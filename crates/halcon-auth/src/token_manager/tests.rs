//! Integration tests del CenzontleTokenManager validando P-AUTH-1..P-AUTH-8.
//!
//! Usa `wiremock` para simular el SSO y contar requests exactamente.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::credential_manager::CredentialManager;
use crate::file_store::FileCredentialStore;
use crate::keystore::KeyStore;
use crate::secret::SecretString;

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Construye un KeyStore respaldado por un FileCredentialStore temporal (sin Keychain).
fn tmp_keystore() -> (Arc<KeyStore>, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let file_store = FileCredentialStore::at(tmp.path().join("creds.json"));
    let mgr = CredentialManager::with_file_store(file_store);
    let ks = Arc::new(KeyStore::from_manager(mgr));
    (ks, tmp)
}

/// Graba tokens iniciales en el keystore (simula usuario logueado).
fn seed_keystore(ks: &KeyStore, access: &str, refresh: &str, expires_at: u64) {
    ks.set_multiple_secrets([
        (KEY_ACCESS_TOKEN, access),
        (KEY_REFRESH_TOKEN, refresh),
        (KEY_EXPIRES_AT, expires_at.to_string().as_str()),
    ])
    .expect("seed keystore");
}

/// Response body estándar exitoso.
fn success_body(access: &str, refresh: &str, expires_in: u64) -> serde_json::Value {
    serde_json::json!({
        "access_token": access,
        "refresh_token": refresh,
        "expires_in": expires_in,
        "token_type": "Bearer"
    })
}

/// Espera a que la cache tenga un token con expires_at razonable (hidratación).
async fn wait_for_cache_populated(mgr: &CenzontleTokenManager, deadline: Duration) {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if mgr.cache.read().await.is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn p_auth_1_singleflight_100_concurrent_requests_yields_1_sso_call() {
    let sso = MockServer::start().await;
    let counter = Arc::new(AtomicU32::new(0));

    let counter_clone = Arc::clone(&counter);
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(move |_: &Request| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            // Simular latency para que concurrencia realmente suceda.
            std::thread::sleep(Duration::from_millis(150));
            ResponseTemplate::new(200)
                .set_body_json(success_body("new_access", "new_refresh_rotated", 900))
        })
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    // Token "expirado" (expires_at = now-1 secs) → dispara refresh inmediato.
    seed_keystore(&ks, "expired_access", "valid_refresh", now_secs().saturating_sub(1));

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    // 100 requests concurrentes.
    let mut handles = Vec::with_capacity(100);
    for _ in 0..100 {
        let m = Arc::clone(&mgr);
        handles.push(tokio::spawn(async move { m.current_token().await }));
    }

    let mut successes = 0usize;
    for h in handles {
        if h.await.unwrap().is_ok() {
            successes += 1;
        }
    }

    assert_eq!(successes, 100, "all 100 tasks should succeed");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "P-AUTH-1: exactly 1 SSO POST for 100 concurrent requests"
    );
}

#[tokio::test]
async fn p_auth_2_hot_path_cache_hit_no_sso_call() {
    let sso = MockServer::start().await;
    let counter = Arc::new(AtomicU32::new(0));

    let counter_clone = Arc::clone(&counter);
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(move |_: &Request| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(success_body("a", "r", 900))
        })
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    // Token válido por 10 min.
    seed_keystore(&ks, "valid_access", "r", now_secs() + 600);

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    // 50 llamadas — todas deben ser cache hits.
    for _ in 0..50 {
        let t = mgr.current_token().await.expect("current_token");
        assert_eq!(t.expose(), "valid_access");
    }

    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "P-AUTH-2: cache hit path NEVER touches SSO"
    );
}

#[tokio::test]
async fn p_auth_3_force_refresh_bypasses_cache() {
    let sso = MockServer::start().await;
    let counter = Arc::new(AtomicU32::new(0));

    let counter_clone = Arc::clone(&counter);
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(move |_: &Request| {
            let n = counter_clone.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(success_body(
                &format!("access_v{}", n + 1),
                "refresh_rotated",
                900,
            ))
        })
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    // Token válido — current_token() sería cache hit, pero force_refresh ignora eso.
    seed_keystore(&ks, "cached_access", "valid_refresh", now_secs() + 600);

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    let cached = mgr.current_token().await.unwrap();
    assert_eq!(cached.expose(), "cached_access");

    let fresh = mgr.force_refresh().await.unwrap();
    assert_eq!(fresh.expose(), "access_v1");

    // Siguiente current_token() debe devolver el refreshed (ya en cache).
    let after_force = mgr.current_token().await.unwrap();
    assert_eq!(after_force.expose(), "access_v1");

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "P-AUTH-3: force_refresh → 1 SSO hit; next current_token hits cache"
    );
}

#[tokio::test]
async fn p_auth_3_401_recovery_via_force_refresh() {
    let sso = MockServer::start().await;
    let counter = Arc::new(AtomicU32::new(0));

    let counter_clone = Arc::clone(&counter);
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(move |_: &Request| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(success_body(
                "fresh_access",
                "fresh_refresh",
                900,
            ))
        })
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    seed_keystore(&ks, "stale_token", "valid_refresh", now_secs() + 600);

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    // Simulación de provider: recibe 401 con "stale_token" → llama force_refresh → retry.
    let new_token = mgr.force_refresh().await.expect("force refresh ok");
    assert_eq!(new_token.expose(), "fresh_access");
}

#[tokio::test]
async fn p_auth_4_invalid_grant_maps_to_refresh_expired() {
    let sso = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "refresh token expired"
        })))
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    seed_keystore(&ks, "x", "rotten_refresh", now_secs().saturating_sub(1));

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    let err = mgr.current_token().await.expect_err("should fail");
    assert!(
        matches!(err, AuthError::RefreshTokenExpired),
        "P-AUTH-4: invalid_grant → RefreshTokenExpired, got {err:?}"
    );
}

#[tokio::test]
async fn p_auth_4_reuse_detected_maps_correctly() {
    let sso = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "refresh token reuse detected by family guard"
        })))
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    seed_keystore(&ks, "x", "stolen_refresh", now_secs().saturating_sub(1));

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    let err = mgr.current_token().await.expect_err("should fail");
    assert!(
        matches!(err, AuthError::RefreshTokenReuseDetected),
        "P-AUTH-4: reuse → RefreshTokenReuseDetected, got {err:?}"
    );
}

#[tokio::test]
async fn p_auth_6_sso_503_is_transient() {
    let sso = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "7"))
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    seed_keystore(&ks, "x", "r", now_secs().saturating_sub(1));

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    let err = mgr.current_token().await.expect_err("503");
    assert!(err.is_transient(), "503 must be transient: {err:?}");
    assert_eq!(err.retry_after(), Some(Duration::from_secs(7)));
}

#[tokio::test]
async fn p_auth_6_sso_429_rate_limited() {
    let sso = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30"))
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    seed_keystore(&ks, "x", "r", now_secs().saturating_sub(1));

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    let err = mgr.current_token().await.expect_err("rate limited");
    assert!(matches!(err, AuthError::RateLimited { retry_after_secs: 30 }));
}

#[tokio::test]
async fn p_auth_7_token_not_in_debug_logs() {
    // Debug de SecretString ya testea redaction; este test valida que
    // el TokenManager nunca loguea el contenido del token por accidente.
    let tok = SecretString::new("super_secret_xyz123".into());
    let dbg = format!("{tok:?}");
    assert!(!dbg.contains("xyz123"), "debug leaked: {dbg}");

    // Evento Refreshed no debe contener el token.
    let ev = TokenEvent::Refreshed {
        new_expires_at: 1_000_000,
        latency_ms: 42,
        reason: RefreshReason::Proactive,
    };
    let ev_str = format!("{ev:?}");
    assert!(!ev_str.contains("xyz123"));
    assert!(!ev_str.to_lowercase().contains("access"));
}

#[tokio::test]
async fn p_auth_8_shutdown_rejects_new_requests() {
    let sso = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(success_body("a", "r", 900)))
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    seed_keystore(&ks, "valid", "r", now_secs() + 600);
    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    // Pre-shutdown: OK.
    mgr.current_token().await.expect("pre-shutdown OK");

    mgr.shutdown().await;

    // Post-shutdown: rechaza.
    let err = mgr.current_token().await.expect_err("should reject");
    assert!(matches!(err, AuthError::ManagerShutdown));

    let err = mgr.force_refresh().await.expect_err("force should reject");
    assert!(matches!(err, AuthError::ManagerShutdown));
}

#[tokio::test]
async fn cold_start_without_login_yields_no_refresh_token() {
    let sso = MockServer::start().await;
    let (ks, _tmp) = tmp_keystore();
    // NO seed: usuario nunca logueado.

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    // Dar tiempo a la hidratación.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let err = mgr.current_token().await.expect_err("no login");
    assert!(matches!(err, AuthError::NoRefreshToken), "got {err:?}");
}

#[tokio::test]
async fn proactive_refresh_when_token_near_expiry() {
    let sso = MockServer::start().await;
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = Arc::clone(&counter);

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(move |_: &Request| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(success_body("renewed", "rot", 900))
        })
        .mount(&sso)
        .await;

    let (ks, _tmp) = tmp_keystore();
    // Token expira en 30s — DENTRO del umbral de 60s → dispara proactive.
    seed_keystore(&ks, "old_access", "r", now_secs() + 30);

    let mgr = CenzontleTokenManager::with_keystore(sso.uri(), ks);
    wait_for_cache_populated(&mgr, Duration::from_secs(2)).await;

    let t = mgr.current_token().await.unwrap();
    assert_eq!(t.expose(), "renewed");
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}
