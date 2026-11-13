//! Politique TLS commune à l'appairage HTTP et au client WebSocket v2.
//!
//! Par défaut, toute URL de serveur doit être chiffrée (`https://` ou `wss://`).
//! Le legacy Electron tolérait un appairage HTTP puis rétrogradait
//! silencieusement la vérification TLS (`rejectUnauthorized: false`) ; ici la
//! dérogation est **explicite**, **journalisée** et **réservée aux builds de
//! développement** (`cfg!(debug_assertions)`), via
//! `LIVECHAT_INSECURE_TLS=1`. Un build release ignore cette variable : le jeton
//! overlay ne peut donc jamais être échangé en clair dans un binaire distribué.

/// Variable d'environnement de dérogation TLS (développement uniquement).
pub(crate) const INSECURE_TLS_ENV: &str = "LIVECHAT_INSECURE_TLS";

/// Indique si la dérogation TLS est active.
///
/// La variable doit valoir `1`, `true`, `yes` ou `on` (insensible à la casse) et
/// le binaire doit être un build debug. Toute activation est journalisée en
/// avertissement pour laisser une trace exploitable.
pub(crate) fn insecure_tls_enabled() -> bool {
    let requested = match std::env::var(INSECURE_TLS_ENV) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    };
    if !requested {
        return false;
    }
    if !cfg!(debug_assertions) {
        tracing::warn!(
            variable = INSECURE_TLS_ENV,
            "dérogation TLS ignorée : les builds release exigent un certificat vérifié"
        );
        return false;
    }
    tracing::warn!(
        variable = INSECURE_TLS_ENV,
        "vérification TLS désactivée (développement uniquement) — ne pas utiliser en production"
    );
    true
}

/// Vérifie qu'un schéma d'URL est acceptable pour un canal porteur de jeton.
///
/// `allow_insecure` est injecté par les tests ; le code de production passe
/// `insecure_tls_enabled()`.
pub(crate) fn require_secure_scheme(
    scheme: &str,
    context: &str,
    allow_insecure: bool,
) -> Result<(), String> {
    match scheme {
        "https" | "wss" => Ok(()),
        "http" | "ws" if allow_insecure => {
            tracing::warn!(context, "schéma non chiffré accepté en développement");
            Ok(())
        }
        "http" | "ws" => Err(format!(
            "{context} non chiffré refusé : utilisez HTTPS/WSS (développement uniquement : {INSECURE_TLS_ENV}=1)"
        )),
        other => Err(format!("schéma {context} non supporté : {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_schemes_are_always_accepted() {
        assert!(require_secure_scheme("https", "serverUrl", false).is_ok());
        assert!(require_secure_scheme("wss", "serverUrl", false).is_ok());
    }

    #[test]
    fn insecure_schemes_are_rejected_without_derogation() {
        let error = require_secure_scheme("http", "serverUrl", false).expect_err("refus http");
        assert!(
            error.contains("HTTPS"),
            "message explicite attendu: {error}"
        );
        assert!(error.contains(INSECURE_TLS_ENV));
        assert!(require_secure_scheme("ws", "serverUrl", false).is_err());
    }

    #[test]
    fn insecure_schemes_are_accepted_with_derogation() {
        assert!(require_secure_scheme("http", "serverUrl", true).is_ok());
        assert!(require_secure_scheme("ws", "serverUrl", true).is_ok());
    }

    #[test]
    fn unsupported_scheme_is_rejected() {
        assert!(require_secure_scheme("ftp", "serverUrl", false).is_err());
    }
}
