//! Sonde de santé enrichie (`GET /health`).
//!
//! ## Contrat
//!
//! Le legacy répondait `{"status":"ok"}` sans aucune vérification réelle
//! (`docs/rust/audit-dynamique.md`). Le serveur v3 expose un état exploitable
//! par le superviseur, sans jamais refuser de démarrer :
//!
//! - la réponse est **toujours HTTP 200** : un état dégradé ne doit pas faire
//!   redémarrer en boucle un serveur dont les clients peuvent encore jouer ;
//! - `db` et `discord` valent `ok`, `down` ou `disabled` ;
//! - `engine` reflète le superviseur de lecture : `disabled` tant qu'aucun
//!   overlay n'a démarré le moteur, `down` si le dernier passage est dégradé ou
//!   arrêté, `ok` sinon ;
//! - `status` vaut `ok` si aucune dépendance obligatoire n'est `down`, sinon
//!   `degraded` (une dépendance `disabled` ne dégrade pas l'état) ;
//! - `activeJobs` reflète le nombre de jobs `PLAYING` en base (lecture unique
//!   qui sert aussi de ping SQLite, avec un délai de garde).
//!
//! ## Discord
//!
//! Le legacy exigeait un token Discord au démarrage. Le serveur v3 démarre sans
//! Discord (exigence d'audit) : [`DiscordProbe`] expose `disabled` quand aucune
//! configuration n'est fournie et `down` quand elle l'est mais que le bot n'est
//! pas encore connecté. La Phase 6 remplacera cette implémentation par l'état
//! réel du client Discord, sans changer `/health`.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use lc_domain::JobStatus;
use lc_playback::{PlaybackEngineState, PlaybackHealth};
use serde::Serialize;

use crate::app::AppState;
use crate::config::Config;

/// Délai maximal accordé au ping SQLite de `/health`.
pub const DATABASE_CHECK_TIMEOUT: Duration = Duration::from_secs(1);

/// État d'une dépendance exposé dans `/health`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    /// Dépendance configurée et opérationnelle.
    Ok,
    /// Dépendance configurée mais indisponible.
    Down,
    /// Dépendance volontairement absente de la configuration.
    Disabled,
}

impl HealthState {
    /// Représentation textuelle stable (`ok`/`down`/`disabled`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Down => "down",
            Self::Disabled => "disabled",
        }
    }
}

/// Sonde d'état d'une dépendance.
///
/// Les Phases 3b à 6 fournissent leurs propres implémentations (client Discord,
/// adaptateur média…) ; `/health` n'a besoin que de ce trait.
pub trait HealthProbe: Send + Sync {
    /// État courant de la dépendance.
    fn state(&self) -> HealthState;
}

/// Sonde Discord de la Phase 3a.
///
/// - configuration absente → [`HealthState::Disabled`] ;
/// - configuration présente mais aucun client connecté (Phase 6 à venir) →
///   [`HealthState::Down`].
#[derive(Debug, Clone, Copy)]
pub struct DiscordProbe {
    configured: bool,
}

impl DiscordProbe {
    /// Construit une sonde à partir de la présence effective de la configuration.
    pub fn new(configured: bool) -> Self {
        Self { configured }
    }

    /// Construit la sonde depuis la configuration du serveur.
    pub fn from_config(config: &Config) -> Self {
        Self::new(config.discord_token.is_some() && config.discord_client_id.is_some())
    }
}

impl HealthProbe for DiscordProbe {
    fn state(&self) -> HealthState {
        if self.configured {
            HealthState::Down
        } else {
            HealthState::Disabled
        }
    }
}

/// Corps de `GET /health`.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// `ok` ou `degraded`.
    pub status: &'static str,
    /// Version du serveur (`CARGO_PKG_VERSION`).
    pub version: &'static str,
    /// Version du protocole applicatif (2).
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u8,
    /// État SQLite.
    pub db: HealthState,
    /// État du superviseur de lecture.
    pub engine: HealthState,
    /// Nombre d'échecs consécutifs du moteur, quand il a démarré.
    #[serde(rename = "engineFailures", skip_serializing_if = "Option::is_none")]
    pub engine_failures: Option<u32>,
    /// État Discord.
    pub discord: HealthState,
    /// Durée de fonctionnement en secondes (≥ 0).
    #[serde(rename = "uptimeSec")]
    pub uptime_sec: i64,
    /// Jobs en cours de lecture (`PLAYING`), si la base est disponible.
    #[serde(rename = "activeJobs")]
    pub active_jobs: Option<i64>,
}

/// Sous-routeur de santé.
pub fn router() -> Router<AppState> {
    Router::new().route("/health", get(health_handler))
}

async fn health_handler(State(state): State<AppState>) -> Response {
    let (db, active_jobs) = check_database(&state).await;
    let (engine, engine_failures) = engine_health(state.admin_hub.playback_health().as_ref());
    let discord = state.discord.state();
    let degraded =
        db == HealthState::Down || discord == HealthState::Down || engine == HealthState::Down;
    let response = HealthResponse {
        status: if degraded { "degraded" } else { "ok" },
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: lc_protocol::PROTOCOL_VERSION,
        db,
        engine,
        engine_failures,
        discord,
        uptime_sec: (state.clock.now() - state.started_at)
            .whole_seconds()
            .max(0),
        active_jobs,
    };
    (StatusCode::OK, Json(response)).into_response()
}

/// Traduit l'état du superviseur en contrat `/health`.
fn engine_health(health: Option<&PlaybackHealth>) -> (HealthState, Option<u32>) {
    let Some(health) = health else {
        return (HealthState::Disabled, None);
    };
    let state = match health.state {
        PlaybackEngineState::Starting | PlaybackEngineState::Running => HealthState::Ok,
        PlaybackEngineState::Degraded | PlaybackEngineState::Stopped => HealthState::Down,
    };
    (state, Some(health.consecutive_failures))
}

/// Interroge SQLite avec un délai de garde ; les erreurs deviennent `down`.
async fn check_database(state: &AppState) -> (HealthState, Option<i64>) {
    let Some(db) = &state.db else {
        return (HealthState::Disabled, None);
    };
    let count = tokio::time::timeout(
        DATABASE_CHECK_TIMEOUT,
        db.playback_jobs().count_by_status(JobStatus::Playing),
    )
    .await;
    match count {
        Ok(Ok(count)) => (HealthState::Ok, Some(count)),
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "échec du contrôle SQLite dans /health");
            (HealthState::Down, None)
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = DATABASE_CHECK_TIMEOUT.as_millis(),
                "délai dépassé lors du contrôle SQLite dans /health"
            );
            (HealthState::Down, None)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::config::Config;
    use crate::http::admin::AdminHub;
    use crate::shutdown::Shutdown;
    use lc_domain::GuildId;
    use lc_protocol::{DeviceSettingsStatePayload, ServerMessage, SessionMode};
    use std::sync::Arc;
    use uuid::Uuid;

    struct FixedPlaybackHub(PlaybackHealth);

    #[async_trait::async_trait]
    impl AdminHub for FixedPlaybackHub {
        fn connected_count(&self) -> usize {
            0
        }

        fn connected_for_guild(&self, _guild_id: &GuildId) -> usize {
            0
        }

        fn is_connected(&self, _client_id: &Uuid) -> bool {
            false
        }

        fn close_client(&self, _client_id: &Uuid, _code: u16) -> bool {
            false
        }

        fn close_client_with_reason(&self, _client_id: &Uuid, _code: u16, _reason: &str) -> bool {
            false
        }

        fn playback_health(&self) -> Option<PlaybackHealth> {
            Some(self.0.clone())
        }

        fn device_settings(&self, _client_id: &Uuid) -> Option<DeviceSettingsStatePayload> {
            None
        }

        fn set_session_mode(&self, _client_id: &Uuid, _mode: SessionMode) -> bool {
            false
        }

        fn broadcast_to_guild(&self, _guild_id: &GuildId, _message: ServerMessage) -> usize {
            0
        }

        fn broadcast_all(&self, _message: ServerMessage) -> usize {
            0
        }

        async fn broadcast_stop(&self, _guild_id: &GuildId) -> usize {
            0
        }

        async fn broadcast_settings(&self, _guild_id: &GuildId) -> usize {
            0
        }
    }

    fn config(values: &[(&str, &str)]) -> Config {
        Config::from_pairs(
            values
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        )
        .expect("configuration de test valide")
    }

    #[test]
    fn health_state_serializes_to_lowercase_contract() {
        assert_eq!(HealthState::Ok.as_str(), "ok");
        assert_eq!(HealthState::Down.as_str(), "down");
        assert_eq!(HealthState::Disabled.as_str(), "disabled");
        assert_eq!(
            serde_json::to_string(&HealthState::Disabled).unwrap(),
            "\"disabled\""
        );
    }

    #[test]
    fn discord_probe_reports_disabled_without_configuration() {
        let probe = DiscordProbe::from_config(&config(&[("API_URL", "http://localhost:3000")]));
        assert_eq!(probe.state(), HealthState::Disabled);
    }

    #[test]
    fn discord_probe_reports_down_when_configured_but_not_connected() {
        let probe = DiscordProbe::from_config(&config(&[
            ("API_URL", "http://localhost:3000"),
            ("DISCORD_TOKEN", "token"),
            ("DISCORD_CLIENT_ID", "123"),
        ]));
        assert_eq!(probe.state(), HealthState::Down);
        assert_eq!(Arc::new(probe).state(), HealthState::Down);
        assert_eq!(DiscordProbe::new(true).state(), HealthState::Down);
        assert_eq!(DiscordProbe::new(false).state(), HealthState::Disabled);
    }

    #[test]
    fn engine_health_is_disabled_before_the_first_overlay() {
        assert_eq!(
            engine_health(None),
            (HealthState::Disabled, None),
            "sans hub moteur, /health ne doit pas dégrader le serveur"
        );
    }

    #[test]
    fn engine_health_reports_degraded_and_recovered_states() {
        let running = PlaybackHealth {
            state: PlaybackEngineState::Running,
            consecutive_failures: 0,
            last_error: None,
        };
        assert_eq!(
            engine_health(Some(&running)),
            (HealthState::Ok, Some(0)),
            "un moteur qui vient de réussir est ok"
        );

        let degraded = PlaybackHealth {
            state: PlaybackEngineState::Degraded,
            consecutive_failures: 3,
            last_error: Some("SQLite indisponible".to_owned()),
        };
        assert_eq!(
            engine_health(Some(&degraded)),
            (HealthState::Down, Some(3)),
            "un moteur dégradé doit être visible comme down"
        );

        let stopped = PlaybackHealth {
            state: PlaybackEngineState::Stopped,
            consecutive_failures: 0,
            last_error: None,
        };
        assert_eq!(
            engine_health(Some(&stopped)),
            (HealthState::Down, Some(0)),
            "un moteur arrêté ne peut plus jouer"
        );
    }

    async fn health_json(health: PlaybackHealth) -> serde_json::Value {
        let state = AppState::new(
            Arc::new(config(&[("API_URL", "http://localhost:3000")])),
            None,
            Shutdown::detached(),
        )
        .with_admin_hub(Arc::new(FixedPlaybackHub(health)));
        let response = health_handler(State(state)).await;
        let body = axum::body::to_bytes(response.into_body(), 1_048_576)
            .await
            .expect("corps /health");
        serde_json::from_slice(&body).expect("JSON /health")
    }

    #[tokio::test]
    async fn health_exposes_degraded_engine_and_recovery() {
        let degraded = health_json(PlaybackHealth {
            state: PlaybackEngineState::Degraded,
            consecutive_failures: 2,
            last_error: Some("SQLite indisponible".to_owned()),
        })
        .await;
        assert_eq!(degraded["status"], "degraded");
        assert_eq!(degraded["engine"], "down");
        assert_eq!(degraded["engineFailures"], 2);

        let recovered = health_json(PlaybackHealth {
            state: PlaybackEngineState::Running,
            consecutive_failures: 0,
            last_error: None,
        })
        .await;
        assert_eq!(recovered["status"], "ok");
        assert_eq!(recovered["engine"], "ok");
        assert_eq!(recovered["engineFailures"], 0);
    }
}
