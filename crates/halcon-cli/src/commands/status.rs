use anyhow::Result;
use halcon_core::types::AppConfig;

use super::auth;

/// Show current Halcon status.
pub async fn run(config: &AppConfig, provider: &str, model: &str) -> Result<()> {
    println!("Halcon CLI v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Provider: {provider}");
    println!("Model:    {model}");
    println!();

    // Check provider availability (env var + OS keychain)
    let providers = &config.models.providers;
    println!("Configured providers:");
    for (name, pc) in providers {
        let status = if pc.enabled { "enabled" } else { "disabled" };
        let needs_key = pc.api_key_env.is_some();
        let has_key = auth::resolve_api_key(name, pc.api_key_env.as_deref()).is_some();
        let key_status = if !needs_key {
            "no key needed"
        } else if has_key {
            "key set"
        } else {
            "key missing"
        };
        println!("  {name}: {status}, {key_status}");
    }

    // Show Cenzontle SSO session status (token-based, not in config.toml).
    // In air-gap mode Cenzontle is excluded regardless of token availability.
    // Show Cenzontle SSO session status (token-based, not in config.toml).
    // In air-gap mode Cenzontle is excluded regardless of token availability.
    let air_gap = std::env::var("HALCON_AIR_GAP")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);
    let cenzontle_token = if air_gap {
        None
    } else {
        std::env::var("CENZONTLE_ACCESS_TOKEN")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| {
                halcon_auth::KeyStore::new("halcon-cli")
                    .get_secret("cenzontle:access_token")
                    .ok()
                    .flatten()
            })
    };
    if cenzontle_token.is_some() {
        let via = if std::env::var("CENZONTLE_ACCESS_TOKEN")
            .ok()
            .filter(|v| !v.is_empty())
            .is_some()
        {
            "env var"
        } else {
            "SSO keychain"
        };
        println!("  cenzontle: enabled, token set ({via})");
    } else {
        println!("  cenzontle: not logged in  (run `halcon login cenzontle` to authenticate)");
    }

    println!();
    println!("Security:");
    println!(
        "  PII detection: {}",
        if config.security.pii_detection {
            "on"
        } else {
            "off"
        }
    );
    println!(
        "  Audit trail:   {}",
        if config.security.audit_enabled {
            "on"
        } else {
            "off"
        }
    );

    Ok(())
}
