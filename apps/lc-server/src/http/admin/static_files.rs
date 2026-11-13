//! Assets statiques du panneau d'administration.
//!
//! Sert le dossier livré en Phase 7a (`static/admin/`) avec `ServeDir` :
//!
//! - `GET /admin` ⇒ `308 /admin/` (pas de contenu sans slash final : les
//!   références CSS/JS de `index.html` sont relatives) ;
//! - `GET /admin/` ⇒ `index.html` (via `append_index_html_on_directories`) ;
//! - `GET /admin/app.js`, `/admin/styles.css` ⇒ fichiers tels quels.
//!
//! Toutes les réponses portent `Cache-Control: no-store` et les en-têtes de
//! sécurité du panneau (CSP stricte, nosniff, no-referrer, X-Frame-Options
//! DENY, Permissions-Policy restrictive) posés par [`super::secure`]. La page
//! est publique (aucune donnée métier) ; l'authentification protège les 13
//! routes `/admin/api/**`.
//!
//! # Résolution du dossier
//!
//! Dans l'ordre, le premier dossier existant gagne :
//!
//! 1. `ADMIN_STATIC_DIR` (variable d'environnement, chemin absolu conseillé) ;
//! 2. `static/admin` relatif au répertoire courant (lancement depuis la racine
//!    du dépôt) ;
//! 3. `static/admin` à côté du binaire (déploiement : binaire + `static/`) ;
//! 4. `static/admin` à deux niveaux au-dessus du binaire (`target/debug` en
//!    développement) ;
//! 5. `$CARGO_MANIFEST_DIR/../../static/admin` (chemin de compilation, filet de
//!    sécurité pour les tests et les exécutions `cargo run`).
//!
//! Si aucun ne correspond, `ServeDir` répond 404 : le panneau est inutilisable
//! mais le serveur démarre (un avertissement est journalisé).

use std::path::PathBuf;

use axum::extract::Request;
use axum::routing::get;
use axum::Router;
use tower_http::services::ServeDir;

use super::secure;
use crate::app::AppState;

/// Variable d'environnement qui force le dossier des assets admin.
const STATIC_DIR_ENV: &str = "ADMIN_STATIC_DIR";

/// Chemin relatif historique des assets admin.
const STATIC_DIR_RELATIVE: &str = "static/admin";

/// Sous-routeur des assets statiques (`/admin` et `/admin/*`).
pub fn router() -> Router<AppState> {
    let directory = resolve_static_dir();
    tracing::debug!(directory = %directory.display(), "assets statiques du panneau admin");

    let serve_dir = ServeDir::new(&directory).append_index_html_on_directories(true);

    let routes = Router::new()
        .route("/admin", get(redirect_to_trailing_slash))
        .nest_service("/admin/", serve_dir);

    secure(routes)
}

/// Redirection permanente vers `/admin/` (308, cf. contrat E15).
async fn redirect_to_trailing_slash() -> impl axum::response::IntoResponse {
    axum::response::Redirect::permanent("/admin/")
}

/// Résout le dossier des assets en testant les emplacements connus.
pub fn resolve_static_dir() -> PathBuf {
    for candidate in candidate_directories() {
        if candidate.is_dir() {
            return candidate;
        }
    }

    tracing::warn!(
        directory = STATIC_DIR_RELATIVE,
        "assets du panneau admin introuvables : la page /admin/ répondra 404"
    );
    PathBuf::from(STATIC_DIR_RELATIVE)
}

/// Emplacements candidats, du plus prioritaire au moins prioritaire.
fn candidate_directories() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(configured) = std::env::var(STATIC_DIR_ENV) {
        let configured = configured.trim();
        if !configured.is_empty() {
            candidates.push(PathBuf::from(configured));
        }
    }

    // Racine du dépôt (ou répertoire de travail du service).
    candidates.push(PathBuf::from(STATIC_DIR_RELATIVE));

    if let Ok(executable) = std::env::current_exe() {
        if let Some(binary_dir) = executable.parent() {
            // Déploiement : binaire et dossier `static/` côte à côte.
            candidates.push(binary_dir.join(STATIC_DIR_RELATIVE));
            // Développement : binaire dans `target/debug[/deps]`.
            candidates.push(binary_dir.join("..").join("..").join(STATIC_DIR_RELATIVE));
        }
    }

    // Filet de sécurité pour les tests et `cargo run` depuis n'importe où.
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        candidates.push(
            PathBuf::from(manifest_dir)
                .join("..")
                .join("..")
                .join(STATIC_DIR_RELATIVE),
        );
    }

    candidates
}

// Le paramètre `Request` n'est utilisé que par la signature `nest_service` ;
// l'import explicite documente le type servi.
#[allow(dead_code)]
fn _request_type_documentation(_request: Request) {}
