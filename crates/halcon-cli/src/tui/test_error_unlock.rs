// Test: Verificar que input se desbloquea después de error

#[cfg(test)]
mod error_unlock_tests {
    use super::super::*;

    #[test]
    fn input_unlocks_after_provider_error() {
        let (ui_tx, ui_rx) = tokio::sync::mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = tokio::sync::mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = tokio::sync::mpsc::unbounded_channel();
        let (perm_tx, _perm_rx) = tokio::sync::mpsc::unbounded_channel();

        let mut app = TuiApp::new(ui_rx, prompt_tx, ctrl_tx, perm_tx);

        // Simular flujo completo de error:
        // 1. Usuario envía prompt
        app.handle_action(input::InputAction::SubmitPrompt);
        assert!(app.state.agent_running, "agent_running debe ser true después de submit");

        // 2. Agent inicia procesamiento
        app.handle_ui_event(UiEvent::AgentStartedPrompt);
        assert!(app.state.agent_running, "agent_running debe seguir en true");

        // 3. Error ocurre (provider sin créditos)
        app.handle_ui_event(UiEvent::Error {
            message: "Your credit balance is too low".to_string(),
            hint: None,
        });

        // 4. Agent finaliza
        app.handle_ui_event(UiEvent::AgentFinishedPrompt);
        app.handle_ui_event(UiEvent::PromptQueueStatus(0));
        app.handle_ui_event(UiEvent::AgentDone);

        // VERIFICAR: Después de AgentDone, el input debe estar desbloqueado
        assert!(!app.state.agent_running, "agent_running debe ser false después de AgentDone");
        assert_eq!(app.state.focus, FocusZone::Prompt, "focus debe estar en Prompt");

        // Verificar que InputState está en Idle
        use crate::tui::input_state::InputState;
        assert_eq!(app.prompt.input_state(), InputState::Idle, "InputState debe ser Idle");

        println!("✅ Test passed: Input unlocks after provider error");
    }
}
