//! `CenzontleTokenManager` — lifecycle manager del access_token de Cenzontle.
//!
//! # Contrato (P-AUTH-1..P-AUTH-8)
//!
//! - **P-AUTH-1 SingleFlight**: N requests concurrentes que necesitan refresh
//!   → 1 POST al SSO.  Implementado con `Mutex` local + re-check tras acquire.
//! - **P-AUTH-2 Hot path**: cache hit no hace I/O, sólo `RwLock::read`.
//! - **P-AUTH-3 401 recovery**: `force_refresh()` bypassa cache.
//! - **P-AUTH-4 Rotation integrity**: persiste el refresh_token rotado.
//! - **P-AUTH-5 Async keystore**: persistencia en spawn, no bloquea hot path.
//! - **P-AUTH-6 Graceful degradation**: errores tipados con retry_after.
//! - **P-AUTH-7 No leakage**: `SecretString` redacta Debug; tokens jamás en logs.
//! - **P-AUTH-8 Clean shutdown**: `shutdown()` termina background loop y rechaza
//!   nuevas operaciones con `AuthError::ManagerShutdown`.
//!
//! # Arquitectura
//!
//! ```text
//!  N requests concurrentes
//!           │
//!           ▼
//!  current_token()  ── cache hit? ──► return SecretString (hot path)
//!           │ cache miss / expired
//!           ▼
//!  refresh_lock.lock()  ◄── singleflight barrier
//!           │
//!           ▼
//!  re-check cache (someone may have refreshed)
//!           │ still needs refresh
//!           ▼
//!  refresh::refresh_at_sso()  ── POST /oauth/token ─►  Zuclubit SSO
//!           │
//!           ▼
//!  update cache + spawn keystore write (async, fire-and-forget)
//!           │
//!           ▼
//!  notify background subscribers (telemetry)
//! ```
//!
//! # NO implementa (delegado a caller)
//! - Retry automático con backoff — es responsabilidad del provider (cenzontle/mod.rs).
//!   El manager expone `AuthError` tipado; provider decide backoff.
//! - Almacenamiento persistente más allá del keystore que ya existe.
//!
//! # NO implementa (deliberadamente fuera de scope, antipremature-optimization)
//! - L0 cache con atomic registers (RwLock es suficiente para un CLI; 100ns
//!   budget era ilusorio: cualquier `async fn` ya no es 100ns por el poll overhead).
//! - Distributed singleflight con Redis (Halcon es single-process CLI).
//! - ML predictor para timing de refresh (no hay dataset; refresh proactivo
//!   simple con umbral fijo cubre 100% de los casos reales).

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::error::AuthError;
use crate::keystore::KeyStore;
use crate::refresh::{refresh_at_sso, RefreshResponse};
use crate::secret::SecretString;

/// Nombre del servicio Keychain (debe coincidir con `halcon-cli::commands::sso`).
const KEYCHAIN_SERVICE: &str = "halcon-cli";
const KEY_ACCESS_TOKEN: &str = "cenzontle:access_token";
const KEY_REFRESH_TOKEN: &str = "cenzontle:refresh_token";
const KEY_EXPIRES_AT: &str = "cenzontle:expires_at";

/// OAuth client_id que el SSO espera para Halcon CLI.
const DEFAULT_CLIENT_ID: &str = "halcon-cli";

/// Umbral de refresh proactivo: si quedan menos de esto, refrescar.
/// 60s es agresivo pero seguro: el round-trip al SSO típico < 500ms;
/// esto evita que una request disparada a los 14:59 de un token de 15:00
/// llegue al SSO con el token ya expirado.
const PROACTIVE_REFRESH_THRESHOLD_SECS: u64 = 60;

/// Interval del background refresh loop.  5 min balancea entre
/// overhead (288 calls/día) y frescura garantizada (token siempre > 10 min).
const BACKGROUND_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Timeout del POST al SSO.  RFC 6749 no impone límite; 20s cubre redes lentas
/// sin volver el UX hang-prone.
const SSO_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

// ─── Trait pública ───────────────────────────────────────────────────────────

/// Interfaz del TokenManager.  Traitificado para permitir mocks en integration tests.
#[async_trait]
pub trait AuthTokenManager: Send + Sync {
    /// Hot path: devuelve access_token válido.
    ///
    /// Si remaining < 60s, refresca antes de devolver.  Singleflight garantiza
    /// que N llamadas concurrentes resultan en 1 POST al SSO.
    async fn current_token(&self) -> Result<SecretString, AuthError>;

    /// Force refresh: invalida cache y refresca incondicionalmente.
    ///
    /// Llamado por el caller cuando recibe 401 pese a tener token "válido"
    /// (e.g. revoked upstream, clock skew).
    async fn force_refresh(&self) -> Result<SecretString, AuthError>;

    /// Subscribe a eventos del lifecycle (telemetry hook).
    fn subscribe(&self) -> broadcast::Receiver<TokenEvent>;

    /// Termina limpiamente el background loop.  Tras shutdown, `current_token`
    /// devuelve `AuthError::ManagerShutdown`.
    async fn shutdown(&self);
}

/// Evento del lifecycle del token (telemetry / observability).
#[derive(Debug, Clone)]
pub enum TokenEvent {
    Refreshed {
        new_expires_at: u64,
        latency_ms: u64,
        reason: RefreshReason,
    },
    RefreshFailed {
        error_class: String,
        retry_after: Option<Duration>,
    },
    NeedsRelogin,
    BackgroundLoopStarted,
    BackgroundLoopStopped,
}

/// Motivo del refresh (para telemetry).
#[derive(Debug, Clone, Copy)]
pub enum RefreshReason {
    /// Cache miss (first access).
    ColdStart,
    /// Token próximo a expirar (< umbral).
    Proactive,
    /// Background loop timer.
    Scheduled,
    /// Forced tras 401 del backend.
    ForcedAfter401,
}

// ─── Implementación ──────────────────────────────────────────────────────────

/// Estado cacheado del token en memoria.  Se lee con `RwLock::read()` en hot path.
#[derive(Clone)]
struct CachedToken {
    access_token: SecretString,
    refresh_token: SecretString,
    /// Unix seconds UTC cuando expira el access_token.
    expires_at: u64,
}

pub struct CenzontleTokenManager {
    /// Cache L1: en memoria, compartida entre hot path + refresh task.
    cache: RwLock<Option<CachedToken>>,

    /// SingleFlight mutex: garantiza que un único refresh HTTP ocurre concurrentemente.
    refresh_lock: Mutex<()>,

    /// Cliente HTTP reusable (pooled connections al SSO).
    http: reqwest::Client,

    /// Base URL del SSO (sin trailing slash).
    sso_url: String,

    /// OAuth client_id.
    client_id: String,

    /// Keystore (macOS Secure Enclave / file fallback).  Compartido por Arc.
    keystore: Arc<KeyStore>,

    /// Flag atómico: `true` cuando shutdown() se invocó.
    shutdown_flag: std::sync::atomic::AtomicBool,

    /// Broadcast channel para eventos telemetría.
    events: broadcast::Sender<TokenEvent>,
}

impl CenzontleTokenManager {
    /// Construye un TokenManager y carga el estado inicial del keystore.
    ///
    /// Si el keystore tiene un refresh_token pero no access_token válido,
    /// la primera llamada a `current_token()` disparará un refresh.
    pub fn new(sso_url: impl Into<String>) -> Arc<Self> {
        Self::with_keystore(sso_url, Arc::new(KeyStore::new(KEYCHAIN_SERVICE)))
    }

    /// Variante inyectable para tests (mock keystore).
    pub fn with_keystore(sso_url: impl Into<String>, keystore: Arc<KeyStore>) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .timeout(SSO_REQUEST_TIMEOUT)
            .pool_max_idle_per_host(4)
            .user_agent(concat!("halcon-auth/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let (tx, _) = broadcast::channel(64);

        let manager = Arc::new(Self {
            cache: RwLock::new(None),
            refresh_lock: Mutex::new(()),
            http,
            sso_url: sso_url.into(),
            client_id: DEFAULT_CLIENT_ID.to_string(),
            keystore,
            shutdown_flag: std::sync::atomic::AtomicBool::new(false),
            events: tx,
        });

        // Hidratar cache desde keystore en spawn (no bloquea caller).
        {
            let mgr = Arc::clone(&manager);
            tokio::spawn(async move {
                if let Err(e) = mgr.hydrate_from_keystore().await {
                    tracing::debug!(error = ?e, "cenzontle token manager: initial hydration failed");
                }
            });
        }

        manager
    }

    /// Permite configurar otro client_id (tests; default es `halcon-cli`).
    pub fn with_client_id(self: Arc<Self>, client_id: impl Into<String>) -> Arc<Self> {
        // Safe-ish: Arc::new para un nuevo manager con client_id cambiado.
        // En producción nunca se llama tras el bootstrap.
        let keystore = Arc::clone(&self.keystore);
        Arc::new(Self {
            cache: RwLock::new(None),
            refresh_lock: Mutex::new(()),
            http: self.http.clone(),
            sso_url: self.sso_url.clone(),
            client_id: client_id.into(),
            keystore,
            shutdown_flag: std::sync::atomic::AtomicBool::new(false),
            events: self.events.clone(),
        })
    }

    /// Lee keystore y popula cache.  No-op si no hay credenciales guardadas.
    async fn hydrate_from_keystore(&self) -> Result<(), AuthError> {
        let access = self
            .keystore
            .get_secret(KEY_ACCESS_TOKEN)
            .map_err(|e| AuthError::KeystoreError(e.to_string()))?;
        let refresh = self
            .keystore
            .get_secret(KEY_REFRESH_TOKEN)
            .map_err(|e| AuthError::KeystoreError(e.to_string()))?;
        let expires_at = self
            .keystore
            .get_secret(KEY_EXPIRES_AT)
            .map_err(|e| AuthError::KeystoreError(e.to_string()))?
            .and_then(|s| s.parse::<u64>().ok());

        let (Some(access), Some(refresh)) = (access, refresh) else {
            tracing::debug!("cenzontle token manager: no tokens in keystore (not logged in)");
            return Ok(());
        };

        // Si no hay expires_at, asumimos expirado → próximo current_token() refresca.
        let expires_at = expires_at.unwrap_or(0);

        let mut slot = self.cache.write().await;
        *slot = Some(CachedToken {
            access_token: SecretString::new(access),
            refresh_token: SecretString::new(refresh),
            expires_at,
        });
        tracing::debug!(
            expires_in_secs = expires_at.saturating_sub(now_secs()),
            "cenzontle token manager: hydrated from keystore"
        );
        Ok(())
    }

    /// Persiste tokens al keystore.  Fire-and-forget: no bloquea caller.
    /// Errores se loguean pero no propagan (persistence es best-effort).
    #[allow(dead_code)] // superseded by inline spawn in `persist_and_refresh`
    fn persist_async(self: &Arc<Self>, refreshed: &RefreshResponse, expires_at: u64) {
        let keystore = Arc::clone(&self.keystore);
        let access = refreshed.access_token.expose().to_string();
        let refresh = refreshed
            .refresh_token
            .as_ref()
            .map(|s| s.expose().to_string());

        tokio::spawn(async move {
            let entries: Vec<(&str, String)> = std::iter::once((KEY_ACCESS_TOKEN, access))
                .chain(refresh.into_iter().map(|r| (KEY_REFRESH_TOKEN, r)))
                .chain(std::iter::once((KEY_EXPIRES_AT, expires_at.to_string())))
                .collect();

            let refs: Vec<(&str, &str)> =
                entries.iter().map(|(k, v)| (*k, v.as_str())).collect();

            if let Err(e) = keystore.set_multiple_secrets(refs.iter().copied()) {
                tracing::warn!(error = %e, "cenzontle token manager: keystore persist failed");
            }
        });
    }

    /// Lógica común de refresh: bajo refresh_lock, re-check cache, llama SSO.
    #[allow(dead_code)] // superseded by free fn `persist_and_refresh`
    async fn perform_refresh_locked(
        self: &Arc<Self>,
        reason: RefreshReason,
        bypass_cache_check: bool,
    ) -> Result<SecretString, AuthError> {
        if self.shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AuthError::ManagerShutdown);
        }

        let _lock = self.refresh_lock.lock().await;

        // P-AUTH-1 SingleFlight: re-chequear cache tras adquirir lock.
        // Otra task puede haber refrescado mientras esperábamos.
        if !bypass_cache_check {
            if let Some(cached) = self.cache.read().await.clone() {
                if cached.expires_at > now_secs() + PROACTIVE_REFRESH_THRESHOLD_SECS {
                    return Ok(cached.access_token);
                }
            }
        }

        // Obtener refresh_token actual.
        let refresh_token = {
            let cached = self.cache.read().await;
            match cached.as_ref() {
                Some(c) => c.refresh_token.clone(),
                None => return Err(AuthError::NoRefreshToken),
            }
        };

        // Llamar al SSO.
        let start = std::time::Instant::now();
        let result = refresh_at_sso(
            &self.http,
            &self.sso_url,
            &self.client_id,
            &refresh_token,
            SSO_REQUEST_TIMEOUT,
        )
        .await;
        let latency_ms = start.elapsed().as_millis() as u64;

        let refreshed = match result {
            Ok(r) => r,
            Err(e) => {
                let retry_after = e.retry_after();
                let _ = self.events.send(TokenEvent::RefreshFailed {
                    error_class: format!("{e:?}")
                        .split_whitespace()
                        .next()
                        .unwrap_or("unknown")
                        .to_string(),
                    retry_after,
                });
                if e.requires_relogin() {
                    let _ = self.events.send(TokenEvent::NeedsRelogin);
                }
                return Err(e);
            }
        };

        // Update cache.
        let expires_at = now_secs() + refreshed.expires_in_secs;
        let new_access = refreshed.access_token.clone();
        let new_refresh = refreshed
            .refresh_token
            .clone()
            .unwrap_or_else(|| refresh_token.clone());

        {
            let mut slot = self.cache.write().await;
            *slot = Some(CachedToken {
                access_token: new_access.clone(),
                refresh_token: new_refresh,
                expires_at,
            });
        }

        // Persist async (P-AUTH-5).
        self.persist_async(&refreshed, expires_at);

        // Telemetry.
        let _ = self.events.send(TokenEvent::Refreshed {
            new_expires_at: expires_at,
            latency_ms,
            reason,
        });
        tracing::info!(
            reason = ?reason,
            latency_ms,
            new_expires_in_secs = refreshed.expires_in_secs,
            "cenzontle token manager: refresh succeeded"
        );

        Ok(new_access)
    }

    /// Arranca el background loop de refresh proactivo.
    /// Devuelve `JoinHandle` para `shutdown()` coordinado.
    pub fn start_background_refresh(self: &Arc<Self>) -> JoinHandle<()> {
        let mgr = Arc::clone(self);
        let _ = mgr.events.send(TokenEvent::BackgroundLoopStarted);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(BACKGROUND_REFRESH_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            // Skip first immediate tick; hydrate_from_keystore ya disparó el cold-start path.
            interval.tick().await;

            loop {
                interval.tick().await;

                if mgr.shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = mgr.events.send(TokenEvent::BackgroundLoopStopped);
                    return;
                }

                // Llamamos current_token() que automáticamente refresca si conviene.
                match mgr.current_token().await {
                    Ok(_) => {
                        tracing::debug!("cenzontle token manager: scheduled check OK");
                    }
                    Err(AuthError::NoRefreshToken) | Err(AuthError::ManagerShutdown) => {
                        // No hay login previo o shutdown — parar background loop.
                        let _ = mgr.events.send(TokenEvent::BackgroundLoopStopped);
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(error = ?e, "cenzontle token manager: scheduled refresh failed");
                        // Continue loop — retry en el siguiente tick.
                    }
                }
            }
        })
    }
}

#[async_trait]
impl AuthTokenManager for CenzontleTokenManager {
    async fn current_token(&self) -> Result<SecretString, AuthError> {
        if self.shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AuthError::ManagerShutdown);
        }

        // Hot path: intenta cache hit sin tocar refresh_lock.
        {
            let cached = self.cache.read().await;
            if let Some(c) = cached.as_ref() {
                if c.expires_at > now_secs() + PROACTIVE_REFRESH_THRESHOLD_SECS {
                    return Ok(c.access_token.clone());
                }
            }
        }

        // Cache miss o próximo a expirar → refresh singleflight.
        let reason = if self.cache.read().await.is_some() {
            RefreshReason::Proactive
        } else {
            RefreshReason::ColdStart
        };
        persist_and_refresh(self, reason, false).await
    }

    async fn force_refresh(&self) -> Result<SecretString, AuthError> {
        persist_and_refresh(self, RefreshReason::ForcedAfter401, true).await
    }

    fn subscribe(&self) -> broadcast::Receiver<TokenEvent> {
        self.events.subscribe()
    }

    async fn shutdown(&self) {
        self.shutdown_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.events.send(TokenEvent::BackgroundLoopStopped);
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Versión `&Self` de `perform_refresh_locked`.  Se extrajo para sortear
/// la limitación de que `AuthTokenManager` es `&self` pero `persist_async`
/// necesitaba `self: &Arc<Self>`.  `persist_async` reescrito para tomar
/// `&Arc<KeyStore>` directamente.
async fn persist_and_refresh(
    mgr: &CenzontleTokenManager,
    reason: RefreshReason,
    bypass_cache_check: bool,
) -> Result<SecretString, AuthError> {
    if mgr.shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(AuthError::ManagerShutdown);
    }

    let _lock = mgr.refresh_lock.lock().await;

    if !bypass_cache_check {
        if let Some(cached) = mgr.cache.read().await.clone() {
            if cached.expires_at > now_secs() + PROACTIVE_REFRESH_THRESHOLD_SECS {
                return Ok(cached.access_token);
            }
        }
    }

    let refresh_token = {
        let cached = mgr.cache.read().await;
        match cached.as_ref() {
            Some(c) => c.refresh_token.clone(),
            None => return Err(AuthError::NoRefreshToken),
        }
    };

    let start = std::time::Instant::now();
    let result = refresh_at_sso(
        &mgr.http,
        &mgr.sso_url,
        &mgr.client_id,
        &refresh_token,
        SSO_REQUEST_TIMEOUT,
    )
    .await;
    let latency_ms = start.elapsed().as_millis() as u64;

    let refreshed = match result {
        Ok(r) => r,
        Err(e) => {
            let retry_after = e.retry_after();
            let _ = mgr.events.send(TokenEvent::RefreshFailed {
                error_class: format!("{e:?}")
                    .split_whitespace()
                    .next()
                    .unwrap_or("unknown")
                    .to_string(),
                retry_after,
            });
            if e.requires_relogin() {
                let _ = mgr.events.send(TokenEvent::NeedsRelogin);
            }
            return Err(e);
        }
    };

    let expires_at = now_secs() + refreshed.expires_in_secs;
    let new_access = refreshed.access_token.clone();
    let new_refresh = refreshed
        .refresh_token
        .clone()
        .unwrap_or_else(|| refresh_token.clone());

    {
        let mut slot = mgr.cache.write().await;
        *slot = Some(CachedToken {
            access_token: new_access.clone(),
            refresh_token: new_refresh,
            expires_at,
        });
    }

    // Persist async — fire-and-forget.
    let keystore = Arc::clone(&mgr.keystore);
    let access_str = refreshed.access_token.expose().to_string();
    let refresh_str = refreshed
        .refresh_token
        .as_ref()
        .map(|s| s.expose().to_string());
    tokio::spawn(async move {
        let mut entries: Vec<(&str, String)> = vec![
            (KEY_ACCESS_TOKEN, access_str),
            (KEY_EXPIRES_AT, expires_at.to_string()),
        ];
        if let Some(r) = refresh_str {
            entries.push((KEY_REFRESH_TOKEN, r));
        }
        let refs: Vec<(&str, &str)> = entries.iter().map(|(k, v)| (*k, v.as_str())).collect();
        if let Err(e) = keystore.set_multiple_secrets(refs.iter().copied()) {
            tracing::warn!(error = %e, "cenzontle token manager: keystore persist failed");
        }
    });

    let _ = mgr.events.send(TokenEvent::Refreshed {
        new_expires_at: expires_at,
        latency_ms,
        reason,
    });
    tracing::info!(
        reason = ?reason,
        latency_ms,
        new_expires_in_secs = refreshed.expires_in_secs,
        "cenzontle token manager: refresh succeeded"
    );

    Ok(new_access)
}

#[cfg(test)]
mod tests;
