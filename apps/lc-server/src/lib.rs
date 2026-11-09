//! Bibliothèque du serveur LiveChat (Phase 3a).
//!
//! Le binaire [`main`] est volontairement mince : tout vit ici, pour que les
//! lanes suivantes (routes overlay/ingest, hub WebSocket, adaptateur
//! `lc-playback`) puissent tester et greffer leurs modules sans se disputer
//! `main.rs`.
//!
//! ## Modules
//!
//! - [`config`] : configuration d'environnement typée et validée (mêmes noms et
//!   valeurs par défaut que le serveur WebSocket legacy) ;
//! - [`app`] : état partagé [`AppState`], assemblage du routeur axum et
//!   ouverture de la base SQLite ;
//! - [`http`] : sous-routeurs et middlewares transverses (sécurité, CORS, rate
//!   limit, normalisation d'erreurs, `/health`) ;
//! - [`shutdown`] : signal d'arrêt partagé, handlers de signaux et délai de
//!   drainage.
//!
//! ## Cycle de vie
//!
//! [`run`] initialise la journalisation, ouvre la base si elle est configurée,
//! lie la socket TCP, puis sert le routeur jusqu'à un signal d'arrêt (`Ctrl+C`
//! ou `SIGTERM`). Le serveur ne dépend d'aucune intégration externe pour
//! démarrer : Discord et SQLite sont facultatifs, et `/health` publie leur état
//! réel (`ok` / `down` / `disabled`).
//!
//! Aucun endpoint de *shutdown* ou de debug n'est exposé : l'arrêt gracieux
//! passe uniquement par les signaux du processus (recommandation du POC).

#![forbid(unsafe_code)]

pub mod app;
pub mod config;
pub mod http;
pub mod media_url;
pub mod shutdown;
pub mod ws;

use std::future::IntoFuture as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

pub use app::{router, AppState};
pub use config::{Config, ConfigError};
pub use shutdown::{Shutdown, ShutdownSignal, DRAIN_TIMEOUT};

/// Adresse d'écoute : toutes les interfaces IPv4.
///
/// Le legacy Node écoutait en dual-stack (`::`) ; on reste volontairement sur
/// `0.0.0.0` pour ne jamais refuser de démarrer quand l'IPv6 est indisponible
/// et parce que le socket IPv6 n'est pas dual-stack sur toutes les plateformes.
/// Conséquence documentée : un client qui résout `localhost` en `::1` d'abord
/// bascule sur IPv4 (comportement standard des clients HTTP) ; le support
/// IPv6 complet relève de la Phase 7 (reverse proxy / installateur).
pub const LISTEN_ADDRESS: Ipv4Addr = Ipv4Addr::UNSPECIFIED;

/// Initialise la journalisation globale (`tracing`).
///
/// Priorité à `RUST_LOG` (format `EnvFilter`) ; à défaut, la variable historique
/// `LOG` (`info` par défaut) est traduite en directive `tracing`
/// (`warning` → `warn`, `silent` → `off`). Une valeur invalide ne fait jamais
/// échouer le démarrage : on retombe sur `info` en le signalant sur `stderr`.
pub fn init_tracing(config: &Config) {
    let filter = match config.rust_log.as_deref() {
        Some(value) => filter_from_directive(value),
        None => filter_from_directive(config.log_level.directive()),
    };

    if let Err(error) = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init()
    {
        eprintln!("initialisation de la journalisation impossible : {error}");
    }
}

/// Construit un [`EnvFilter`] sans panique, avec repli `info` tracé sur `stderr`.
fn filter_from_directive(directive: &str) -> EnvFilter {
    match EnvFilter::try_new(directive) {
        Ok(filter) => filter,
        Err(error) => {
            eprintln!(
                "directive de journalisation invalide ({directive}) : {error} — utilisation de info"
            );
            // « info » est une directive statique valide, ce chemin ne peut pas
            // échouer ; `try_new` évite néanmoins tout panic théorique.
            EnvFilter::try_new("info").unwrap_or_default()
        }
    }
}

/// Démarre le serveur puis attend son arrêt gracieux.
///
/// Retourne une erreur si la configuration d'exécution est inutilisable (base
/// configurée mais inaccessible, port déjà pris…) ; `main` la convertit en code
/// de sortie 1. Aucun `timeout` global n'enveloppe le service : un serveur lent à
/// drainer doit pouvoir terminer ses connexions (recommandation du POC).
pub async fn run(config: Config) -> anyhow::Result<()> {
    init_tracing(&config);
    serve(config).await
}

/// Cœur du démarrage : ouverture des dépendances, bind, service, drainage.
async fn serve(config: Config) -> anyhow::Result<()> {
    let started = Instant::now();
    let config = Arc::new(config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        protocol_version = lc_protocol::PROTOCOL_VERSION,
        node_env = %config.node_env,
        port = config.port,
        "démarrage du serveur LiveChat"
    );

    let (shutdown, signal) = Shutdown::new();
    shutdown::install_signal_handlers(shutdown.clone());

    // Base facultative : `open_database` ne renvoie une erreur que si
    // `DATABASE_URL` est fournie et inutilisable (échec explicite, pas de perte
    // silencieuse).
    let database = app::open_database(&config).await?;
    let database_for_shutdown = database.clone();
    let state = AppState::new(Arc::clone(&config), database, shutdown.clone());
    let router = app::router(state);

    let listener = bind_listener(config.port).await?;
    let local_address = listener
        .local_addr()
        .context("adresse d'écoute locale illisible")?;

    // Mesure réelle du démarrage : de l'entrée dans `serve` jusqu'à l'écoute
    // effective (ouverture SQLite incluse). Affichée en millisecondes.
    info!(
        address = %local_address,
        startup_ms = started.elapsed().as_millis(),
        "serveur HTTP à l'écoute"
    );

    let server = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(signal.wait())
    .into_future();
    tokio::pin!(server);

    let drain = wait_for_drain(shutdown.clone());
    tokio::pin!(drain);

    let server_result = tokio::select! {
        result = &mut server => Some(result),
        () = &mut drain => None,
    };
    shutdown.trigger();

    if let Some(database) = database_for_shutdown {
        database.close().await;
        info!("base SQLite fermée");
    }

    match server_result {
        Some(result) => {
            result.context("erreur du serveur HTTP")?;
            info!(
                uptime_ms = started.elapsed().as_millis(),
                "serveur HTTP arrêté proprement"
            );
        }
        None => warn!(
            drain_timeout_ms = shutdown::DRAIN_TIMEOUT.as_millis(),
            "délai de drainage des connexions dépassé : arrêt forcé"
        ),
    }

    Ok(())
}

/// Lie la socket d'écoute IPv4 (toutes interfaces).
///
/// Le port occupé est le seul échec possible et il est remonté explicitement à
/// `run()` (code de sortie 1), avec le contexte de la variable `PORT`.
async fn bind_listener(port: u16) -> anyhow::Result<TcpListener> {
    let address = SocketAddr::new(IpAddr::V4(LISTEN_ADDRESS), port);
    TcpListener::bind(address)
        .await
        .with_context(|| format!("écoute HTTP impossible sur {address}"))
}

/// Attend le signal d'arrêt puis laisse au serveur le temps de drainer.
async fn wait_for_drain(shutdown: Shutdown) {
    shutdown.subscribe().wait().await;
    sleep(DRAIN_TIMEOUT).await;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::net::SocketAddr;
    use std::time::Duration;

    use tokio::net::TcpListener;

    use super::*;

    fn test_config() -> Config {
        let pairs = [("API_URL".to_owned(), "http://localhost:3000".to_owned())];
        Config::from_pairs(pairs).expect("configuration de test valide")
    }

    #[tokio::test]
    async fn shutdown_signal_stops_the_http_server_cleanly() {
        let (shutdown, signal) = Shutdown::new();
        let state = AppState::new(Arc::new(test_config()), None, shutdown.clone());
        let router = app::router(state);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("socket de test");
        let server = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(signal.wait())
        .into_future();
        let handle = tokio::spawn(server);
        tokio::task::yield_now().await;

        shutdown.trigger();

        let result = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("le serveur doit s'arrêter dans les délais")
            .expect("la tâche de service ne doit pas paniquer");
        result.expect("arrêt gracieux sans erreur");
    }
}
