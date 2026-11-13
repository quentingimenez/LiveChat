//! Authentification des routes `/overlay/media-board/*`.
//!
//! ## Contrat v2
//!
//! Le legacy (`server/src/services/overlayAuth.ts:116-143`) acceptait le token
//! en en-tête **ou** dans `?token=`. Le protocole v2 supprime le passage par
//! query string : les routes REST du board exigent
//! `Authorization: Bearer <token>`, et le flux binaire
//! `GET /overlay/media-board/media/{assetId}` est authentifié par une URL
//! signée (`clientId`, `exp`, `sig`, cf. [`super::media_board::get_media`]).
//! Aucune route overlay n'accepte plus de jeton en query string.
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
    /// Membre de guilde propriétaire de l'overlay (migration 0003).
    ///
    /// `None` pour un overlay sans membre rattaché : son rôle se déduit alors
    /// du mode de session persisté (cf. [`AuthedOverlayClient::role`]).
    pub member: Option<lc_domain::GuildMember>,
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

    /// Rôle du client dans sa guilde.
    ///
    /// Celui du membre rattaché ; à défaut, l'équivalent du mode de session
    /// persisté (`invite_read_only` → invité, `normal` → membre). Comme le mode,
    /// il vient toujours de la base, jamais de la requête.
    pub fn role(&self) -> lc_domain::GuildRole {
        match &self.member {
            Some(member) => member.role,
            None => match self.client.session_mode {
                lc_domain::OverlaySessionMode::InviteReadOnly => lc_domain::GuildRole::Guest,
                lc_domain::OverlaySessionMode::Normal => lc_domain::GuildRole::Member,
            },
        }
    }

    /// `true` si le rôle du client autorise `permission`.
    pub fn can(&self, permission: lc_domain::Permission) -> bool {
        self.role().can(permission)
    }

    /// Identifiant du membre rattaché, s'il existe.
    pub fn member_id(&self) -> Option<lc_domain::GuildMemberId> {
        self.member.as_ref().map(|member| member.id)
    }

    /// `true` si la session est en lecture seule (rôle invité).
    ///
    /// Les routes qui écrivent (board média : `POST`, `PATCH`, `DELETE`) doivent
    /// refuser la requête avec `403 {"error":"read_only_session"}` lorsque
    /// cette méthode renvoie `true` (cf. `media_board.rs`,
    /// `ensure_writable`). Les routes de lecture (`GET`) restent autorisées.
    pub fn is_read_only(&self) -> bool {
        self.role() == lc_domain::GuildRole::Guest
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
        // Un seul mécanisme : l'en-tête `Authorization: Bearer`. Aucun repli
        // `?token=` (supprimé avec AUD-COR-05).
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

            let member = match db.guild_members().get_for_overlay_client(client_id).await {
                Ok(member) => member,
                Err(error) => return Err(db_failure("overlay_auth.member_lookup", &error)),
            };

            Ok(Self {
                client,
                member,
                authenticated_at: now,
            })
        }
    }
}
