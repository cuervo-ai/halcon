# Análisis Detallado del TUI - Problemas Detectados

## 📊 Resumen Ejecutivo

Basado en el análisis sistemático del código y los resultados que proporcionaste, he identificado **5 problemas críticos** en el TUI de cuervo-cli que explican TODOS los comportamientos observados:

---

## 🐛 Problema #1: Input Bloqueado Después de AgentDone

### Síntomas Observados:
```
Estado del TUI:
- Status bar: "⏳ AWAIT"
- Botón: "▶ Processing"
- Prompt: placeholder visible ("Type your message here...")
- Input NO responde a teclas
```

### Causa Raíz:

El problema está en la desincronización del estado `prompts_queued`. Cuando el agente termina:

**Flujo Actual:**
1. Usuario envía prompt → `prompts_queued++` (optimista, línea 819)
2. Agent loop dequeue → `AgentStartedPrompt` → `prompts_queued--` (línea 1038)
3. Agent procesa y termina → `AgentFinishedPrompt` → **NO decrementa** (línea 1041-1044)
4. Luego envía `PromptQueueStatus(count)` → actualiza contador (línea 1048)

**El Problema:**
Si hay lag entre `AgentFinishedPrompt` y `PromptQueueStatus`, el TUI queda en un estado inconsistente:
- `agent_running = false` (correcto)
- `prompts_queued > 0` (INCORRECTO si la cola está vacía)
- Botón muestra "Processing" aunque el agente está idle

### Código Problemático:

```rust
// app.rs:278-293
let (btn_text, ...) = if self.state.prompts_queued > 0 {
    // Agent is processing or prompts queued
    let text = if self.state.prompts_queued == 1 {
        "  ▶ Processing  "  // ← Usuario ve esto
    } else {
        "  ⏳ Queued (#N)  "
    };
    // ...
} else {
    // Ready for new prompt
    "  ► Send (Ctrl+⏎)  " // ← Debería ver esto
    // ...
}
```

**El input NO está bloqueado programáticamente** - el problema es que el usuario PERCIBE que está bloqueado porque el botón dice "Processing".

---

## 🐛 Problema #2: Error de Terminal Corrupto

### Síntomas Observados:
```
Error: input failed — The cursor position could not be read within a normal duration
```

### Causa Raíz:

Este error ocurre cuando:
1. El terminal entra/sale de raw mode de forma incorrecta
2. Hay un error en el cleanup del terminal al salir del TUI
3. El terminal está en un estado inconsistente después de Ctrl+C o crash

### Código Relevante:

```rust
// app.rs:172-186 - Setup
stdout.execute(EnterAlternateScreen)?;
terminal::enable_raw_mode()?;
stdout.execute(EnableMouseCapture)?;
stdout.execute(PushKeyboardEnhancementFlags(...))?;

// Al salir (cleanup implícito en Drop?)
// NO HAY cleanup explícito visible en el código mostrado
```

**El Problema:**
Si el TUI se cierra abruptamente (ej: panic, Ctrl+C mal manejado), el terminal queda en raw mode sin restaurar el estado original.

---

## 🐛 Problema #3: Primera Ejecución Sin Interacción (0 rounds)

### Síntomas Observados:
```bash
# Primera ejecución en cuervo-cli/
Session Summary:
  Rounds: 0 | Tokens: ↑0K ↓0K | Cost: $0.0000
  Duration: 0.0s | Tools: 0 calls | Total tokens: 0
```

### Causa Probable:

El usuario abrió el TUI pero NO pudo enviar ningún prompt. Posibles causas:
1. **No supo cómo enviar**: El TUI no indica claramente que es Ctrl+Enter (solo lo muestra en el botón)
2. **Presionó Enter en vez de Ctrl+Enter**: Enter inserta nueva línea, no envía
3. **Problema con el foco**: El foco no estaba en el prompt (unlikely, el default es Prompt)

---

## 🐛 Problema #4: Estado "⏳ AWAIT" Indefinido

### Síntomas Observados:
```
Status bar muestra: "⏳ AWAIT │ SESSION │ anthropic/claude-haiku... │ R5 │..."
```

### Causa Raíz CONFIRMADA:

El símbolo "⏳ AWAIT" proviene de `AgentControl::WaitingApproval`:

```rust
// status.rs:188-193
let (ctrl_label, ctrl_color) = match self.agent_control {
    AgentControl::Running => ("▶ RUN", c_success),
    AgentControl::Paused => ("⏸ PAUSE", c_warning),
    AgentControl::StepMode => ("⏭ STEP", c_accent),
    AgentControl::WaitingApproval => ("⏳ AWAIT", c_planning), // ← Este es el culpable
};
```

**El Problema:**
El TUI está atascado en estado `WaitingApproval` cuando NO debería estarlo. Esto ocurre cuando:
1. Se abre un overlay de permisos (PermissionPrompt)
2. El overlay se cierra pero el estado `agent_control` NO se resetea a `Running`
3. El TUI queda bloqueado esperando aprobación que nunca llega

**Código Problemático:**
```rust
// app.rs:600-636 - handle_overlay_key()
KeyCode::Enter => {
    match &self.state.overlay.active {
        Some(OverlayKind::PermissionPrompt { .. }) => {
            let _ = self.perm_tx.send(true);
            self.activity.push_info("[control] Action approved");
            self.state.overlay.close(); // ← Cierra overlay
            // ❌ FALTA: self.state.agent_control = AgentControl::Running;
        }
        // ...
    }
}
```

Cuando el usuario cierra el overlay de permisos (Enter o Esc), el overlay se cierra pero `agent_control` permanece en `WaitingApproval`, dejando el TUI en un estado zombi.

---

## 🐛 Problema #5: Botón de Envío Bloqueado por Click de Mouse

### Código Problemático:

```rust
// app.rs:441-448
MouseEventKind::Down(MouseButton::Left) => {
    let r = self.submit_button_area;
    if !self.state.agent_running  // ← Solo funciona si agent NO está running
        && mouse.column >= r.x
        && mouse.column < r.x + r.width
        && mouse.row >= r.y
        && mouse.row < r.y + r.height
    {
        self.handle_action(input::InputAction::SubmitPrompt);
    }
}
```

**El Problema:**
El botón solo responde a clicks cuando `agent_running = false`. Pero si:
- `agent_running = false` (el agente terminó)
- `prompts_queued > 0` (desincronización del Problema #1)

Entonces:
- El botón muestra "Processing" (porque `prompts_queued > 0`)
- PERO el click NO funciona (porque `agent_running = false`)

Esto crea una experiencia confusa donde el botón parece clickeable pero no responde.

---

## 💡 Soluciones Propuestas

### Fix #1: Sincronizar prompts_queued correctamente
```rust
// En handle_ui_event()
UiEvent::AgentFinishedPrompt => {
    // Decrementar inmediatamente si la cola está vacía
    if self.state.prompts_queued > 0 {
        self.state.prompts_queued -= 1;
    }
    tracing::debug!("Agent finished processing prompt");
}
```

### Fix #2: Cleanup explícito del terminal
```rust
impl Drop for TuiApp {
    fn drop(&mut self) {
        // Asegurar que el terminal se restaure
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().execute(LeaveAlternateScreen);
        let _ = io::stdout().execute(DisableMouseCapture);
        let _ = io::stdout().execute(PopKeyboardEnhancementFlags);
    }
}
```

### Fix #3: Mejor indicación visual de cómo enviar
```rust
// En prompt.rs:render()
// Cambiar placeholder a:
"Type your message... (Ctrl+Enter to send, Enter for new line)"
```

### Fix #4: Resetear agent_control al cerrar overlay
```rust
// app.rs:600-636 - En handle_overlay_key()
KeyCode::Esc => {
    if matches!(self.state.overlay.active, Some(OverlayKind::Search)) {
        self.search_matches.clear();
        self.search_current = 0;
    }
    // ✅ AGREGAR: Resetear control state si se cierra permission prompt
    if matches!(self.state.overlay.active, Some(OverlayKind::PermissionPrompt { .. })) {
        self.state.agent_control = AgentControl::Running;
    }
    self.state.overlay.close();
}

KeyCode::Enter => {
    match &self.state.overlay.active {
        Some(OverlayKind::PermissionPrompt { .. }) => {
            let _ = self.perm_tx.send(true);
            self.activity.push_info("[control] Action approved");
            self.state.overlay.close();
            self.state.agent_control = AgentControl::Running; // ✅ AGREGAR
        }
        // ...
    }
}

// También para 'Y' y 'N' key handlers (líneas 657-673)
KeyCode::Char('y') | KeyCode::Char('Y') => {
    if matches!(self.state.overlay.active, Some(OverlayKind::PermissionPrompt { .. })) {
        let _ = self.perm_tx.send(true);
        self.activity.push_info("[control] Action approved");
        self.state.agent_control = AgentControl::Running; // ✅ AGREGAR
        self.state.overlay.close();
    }
    // ...
}

KeyCode::Char('n') | KeyCode::Char('N') => {
    if matches!(self.state.overlay.active, Some(OverlayKind::PermissionPrompt { .. })) {
        let _ = self.perm_tx.send(false);
        self.activity.push_warning("[control] Action rejected", None);
        self.state.agent_control = AgentControl::Running; // ✅ AGREGAR
        self.state.overlay.close();
    }
    // ...
}
```

### Fix #5: Botón clickeable basado en estado real
```rust
// app.rs:441-448 - Cambiar condición del botón
MouseEventKind::Down(MouseButton::Left) => {
    let r = self.submit_button_area;
    // ✅ Permitir click si NO está procesando (basado en prompts_queued)
    if self.state.prompts_queued == 0  // ← Cambiar a esta condición
        && mouse.column >= r.x
        && mouse.column < r.x + r.width
        && mouse.row >= r.y
        && mouse.row < r.y + r.height
    {
        self.handle_action(input::InputAction::SubmitPrompt);
    }
}
```

### Fix #6: Logging mejorado para debugging
```rust
// Agregar tracing en puntos clave:
tracing::debug!(
    agent_running = self.state.agent_running,
    prompts_queued = self.state.prompts_queued,
    agent_control = ?self.state.agent_control,
    focus = ?self.state.focus,
    "TUI state snapshot"
);
```

---

## 🧪 Tests Recomendados

1. **Test de estado consistente después de AgentDone**
2. **Test de cleanup del terminal en panic**
3. **Test de queue vacía pero prompts_queued > 0**
4. **Test de múltiples prompts encolados**

---

## 📌 Siguiente Paso

¿Quieres que:
1. Lea `widgets/status.rs` para confirmar el problema #4?
2. Implemente los fixes propuestos?
3. Cree tests para validar los fixes?
4. Todo lo anterior?
