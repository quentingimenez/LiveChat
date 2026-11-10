//! Authentification des routes `/overlay/meme-board/*` (Bearer uniquement).
//!
//! ## Contrat v2
//!
//! Le legacy (`server/src/services/overlayAuth.ts:116-143`) acceptait le token
//! en en-tête **ou** dans `?token=` ; le protocole v2 supprime le passage par
//! query string (cf. `docs/rust/protocol-v2.md` §9) : seule l'en-tête
//! `Authorization: Bearer <token>` est acceptée. Un `?token=` présent est donc
//! simplement ignoré, ce qui produit un `401` si aucun Bearer valide n'est fourni.
//!
//! Le token brut n'est jamais stocké : seule son empreinte `SHA-256` hexadécimale
//! est persistée dans `overlay_clients.token_hash` (parité legacy
//! `hashOverlayToken`, `server/src/services/overlayAuth.ts:36-38`). Les jetons
//! sont générés par [`generate_token`] : 32 octets aléatoires encodés en hex
//! (64 caractères), équivalent de `crypto.randomBytes(32).toString('hex')`.
//!
//! ## Extracteur d'authentification
//!
//! L'authentification est portée par l'extracteur [`AuthedOverlayClient`]
//! (`FromRequestParts<AppState>`) : il s'exécute avant le handler de chaque route
//! protégée, vérifie le Bearer, charge le client non révoqué et rafraîchit
//! `last_seen_at` de façon best-effort (`OverlayClientsRepository::touch_last_seen`,
//! un échec est journalisé mais ne fait pas échouer la requête).
//!
//! Historiquement un middleware `from_fn` posait le client dans les extensions
//! de requête ; l'extracteur est strictement équivalent ici et évite de dépendre
//! de l'état du routeur parent (le sous-routeur `/overlay` est complété par
//! `crate::app::router`, qui seul détient l'instance `AppState`).

use std::future::Future;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, HeaderMap};
use axum::response::Response;
use chrono::{DateTime, Utc};
use rand::RngCore as _;
use sha2::{Digest as _, Sha256};
use tracing::{debug, warn};

use crate::app::AppState;

use super::{database_unavailable, db_failure, unauthorized};

/// Client overlay authentifié, produit par l'extracteur du même nom.
#[derive(Clone, Debug)]
pub struct AuthedOverlayClient {
    /// Client tel que persisté (`overlay_clients`), forcément non révoqué.
    pub client: lc_domain::OverlayClient,
    /// Instant d'authentification (horloge injectée).
    /// Horodatage de la résolution du client (conservé pour l'audit et les
    /// handlers qui voudront borner la fraîcheur d'un token).
    #[allow(dead_code)]
    pub authenticated_at: DateTime<Utc>,
}

impl AuthedOverlayClient {
    /// Raccourci pratique pour les handlers.
    pub fn guild_id(&self) -> &lc_domain::GuildId {
        &self.client.guild_id
    }
}

/// Génère un token overlay : 32 octets aléatoires en hexadécimal minuscule.
pub fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Empreinte stockée en base pour un token brut (`SHA-256` hex minuscule).
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

/// Extrait le token d'un en-tête `Authorization: Bearer <token>`.
///
/// Le nom du schéma est comparé sans tenir compte de la casse (parité legacy
/// `getBearerTokenFromRequest`). Un token vide est refusé.
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let mut parts = raw.split_whitespace();
    let scheme = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = parts.next()?;
    if token.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(token.to_owned())
}

impl FromRequestParts<AppState> for AuthedOverlayClient {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let token = bearer_token(&parts.headers);
        let db = state.db.clone();
        let now = state.clock.now();
        async move {
            let Some(token) = token else {
                return Err(unauthorized());
            };
            let Some(db) = db else {
                return Err(database_unavailable());
            };
            let token_hash = hash_token(&token);
            let client = match db.overlay_clients().get_by_token_hash(&token_hash).await {
                Ok(Some(client)) => client,
                Ok(None) => {
                    debug!("jeton overlay inconnu : authentification refusée");
                    return Err(unauthorized());
                }
                Err(error) => return Err(db_failure("overlay_auth.client_lookup", &error)),
            };
            let client_id = client.id;

            // Mise à jour best-effort : ne doit jamais faire échouer la requête.
            if let Err(error) = db.overlay_clients().touch_last_seen(client_id, now).await {
                warn!(error = %error, "mise à jour de last_seen_at impossible");
            }

            Ok(Self {
                client,
                authenticated_at: now,
            })
        }
    }
}
