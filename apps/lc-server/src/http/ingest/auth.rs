//! Authentification des routes `/ingest`.
//!
//! Le legacy (`server/src/services/ingestAuth.ts`) n'accepte que les clients
//! d'ingestion créés par appairage : token en clair fourni en
//! `Authorization: Bearer`, hash SHA-256 comparé à `ingest_clients.token_hash`
//! (les clients révoqués sont exclus par le repository). `INGEST_API_TOKEN`
//! n'est **jamais** utilisé pour authentifier une requête d'ingestion (il ne
//! sert qu'à la configuration admin) ; il est conservé pour parité.
//!
//! Le statut `503` est réservé au cas où l'ingestion est désactivée : aucun
//! client actif n'existe (aucun appairage encore effectué) ou la base est
//! absente.

use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use lc_db::Db;
use lc_domain::entities::IngestClient;

use crate::app::AppState;
use crate::media::errors::MediaServiceResult;
use crate::media::tokens::hash_ingest_token;

use super::{expired_token_error, media_service_error, simple_error};

/// Résultat de la résolution d'authentification.
#[derive(Debug)]
pub enum IngestAuth {
    /// Requête authentifiée par un client d'ingestion actif.
    Client(IngestClient),
    /// Token trouvé mais client inactif depuis trop longtemps (ou révoqué).
    /// Le heartbeat ne doit pas rafraîchir ce client : un ré-appairage depuis
    /// la board est nécessaire.
    Expired,
    /// Aucune authentification valide ; `enabled` indique si l'ingestion est
    /// configurée (au moins un client actif), pour choisir entre 401 et 503.
    Missing { enabled: bool },
}

/// Décide de l'authentification d'un client trouvé par hash de token.
///
/// Fonction pure (aucune écriture) pour que l'inactivité soit testable sans
/// serveur : un client non utilisable à l'instant `now` devient `Expired` et
/// ne doit jamais être renvoyé comme `Client` (donc jamais « touché »).
pub(super) fn classify_client(client: IngestClient, now: time::UtcDateTime) -> IngestAuth {
    if client.is_usable_at(now) {
        IngestAuth::Client(client)
    } else {
        IngestAuth::Expired
    }
}

/// Extrait le token `Bearer` d'un en-tête `Authorization`.
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = raw.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

/// Résout l'authentification d'une requête d'ingestion.
pub async fn resolve(state: &AppState, headers: &HeaderMap) -> MediaServiceResult<IngestAuth> {
    let Some(db) = state.db.as_ref() else {
        return Ok(IngestAuth::Missing { enabled: false });
    };

    let Some(token) = bearer_token(headers) else {
        return Ok(IngestAuth::Missing {
            enabled: client_is_enabled(db).await?,
        });
    };

    let token_hash = hash_ingest_token(token);
    match db.ingest_clients().get_by_token_hash(&token_hash).await? {
        Some(client) => {
            // L'expiration est calculée par inactivité (aucune migration) :
            // un client inutilisable est refusé AVANT tout heartbeat, pour ne
            // pas le ressusciter par une simple écriture de `last_seen_at`.
            match classify_client(client, state.clock.now()) {
                IngestAuth::Client(client) => {
                    // Heartbeat applicatif : une erreur d'écriture ne doit pas
                    // refuser une requête authentifiée valide.
                    if let Err(error) = db
                        .ingest_clients()
                        .touch_last_seen(client.id, state.clock.now())
                        .await
                    {
                        tracing::warn!(
                            error = %error,
                            client_id = %client.id.to_canonical(),
                            "impossible de mettre à jour last_seen_at du client d'ingestion"
                        );
                    }
                    Ok(IngestAuth::Client(client))
                }
                other => Ok(other),
            }
        }
        None => Ok(IngestAuth::Missing {
            enabled: client_is_enabled(db).await?,
        }),
    }
}

/// Résout le client d'ingestion d'une requête, ou la réponse d'erreur à
/// renvoyer.
///
/// Centralise le mapping [`IngestAuth`] → réponse pour que `POST /ingest`,
/// `POST /ingest/file` et `GET /ingest/whoami` répondent exactement le même
/// statut et le même code à un jeton inconnu (`401 unauthorized`), expiré
/// (`401 ingest_token_expired`) ou à une ingestion désactivée
/// (`503 ingest_api_disabled`). Un client authentifié a déjà vu son
/// `last_seen_at` rafraîchi par [`resolve`].
pub async fn require_client(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<IngestClient, Response> {
    match resolve(state, headers).await {
        Ok(IngestAuth::Client(client)) => Ok(client),
        Ok(IngestAuth::Expired) => Err(expired_token_error()),
        Ok(IngestAuth::Missing { enabled: true }) => {
            Err(simple_error(StatusCode::UNAUTHORIZED, "unauthorized"))
        }
        Ok(IngestAuth::Missing { enabled: false }) => Err(simple_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "ingest_api_disabled",
        )),
        Err(error) => Err(media_service_error(error)),
    }
}

/// `true` si au moins un client d'ingestion non révoqué existe.
///
/// L'expiration par inactivité (C-04) n'entre pas en compte : elle vise un
/// jeton précis, pas la configuration du serveur. Avec uniquement des clients
/// expirés, l'ingestion reste « configurée » : une requête sans jeton reçoit
/// `401 unauthorized` (et non `503 ingest_api_disabled`), une requête avec
/// un jeton expiré reçoit `401 ingest_token_expired`.
pub async fn client_is_enabled(db: &Db) -> MediaServiceResult<bool> {
    let clients = db.ingest_clients().list().await?;
    Ok(clients.iter().any(|client| client.is_active()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn expired_client_is_never_authenticated_and_keeps_its_activity() {
        use lc_domain::entities::INGEST_CLIENT_IDLE_EXPIRY_MS;
        use lc_domain::ids::{GuildId, IngestClientId};
        use lc_domain::ms_to_datetime;
        use time::UtcDateTime;

        let client =
            |last_seen_at: Option<UtcDateTime>, revoked_at: Option<UtcDateTime>| IngestClient {
                id: IngestClientId::new_v4(),
                guild_id: GuildId::parse("123456789012345678").unwrap(),
                label: "Extension".to_owned(),
                token_hash: "hash".to_owned(),
                default_author_name: "LiveChat Extension".to_owned(),
                default_author_image: None,
                created_by_discord_user_id: "42".to_owned(),
                last_seen_at,
                created_at: ms_to_datetime(1_700_000_000_000).unwrap(),
                revoked_at,
            };

        let created = ms_to_datetime(1_700_000_000_000).unwrap();
        let now = ms_to_datetime(1_700_000_000_000 + INGEST_CLIENT_IDLE_EXPIRY_MS).unwrap();
        // Inactif depuis exactement le TTL : expiré, aucune écriture possible.
        assert!(matches!(
            classify_client(client(None, None), now),
            IngestAuth::Expired
        ));
        // TTL - 1 ms : encore authentifié.
        let fresh = ms_to_datetime(1_700_000_000_000 + INGEST_CLIENT_IDLE_EXPIRY_MS - 1).unwrap();
        assert!(matches!(
            classify_client(client(Some(created), None), fresh),
            IngestAuth::Client(_)
        ));
        // Un heartbeat récent remet l'échéance à plus tard.
        let active = client(Some(now), None);
        assert!(matches!(
            classify_client(
                active,
                ms_to_datetime(1_700_000_000_000 + 2 * INGEST_CLIENT_IDLE_EXPIRY_MS - 1).unwrap()
            ),
            IngestAuth::Client(_)
        ));
        // Révoqué : expiré, même avec une activité récente.
        assert!(matches!(
            classify_client(client(Some(now), Some(created)), now),
            IngestAuth::Expired
        ));
    }

    #[test]
    fn extracts_only_bearer_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer 0123456789abcdef"),
        );
        assert_eq!(bearer_token(&headers), Some("0123456789abcdef"));

        headers.insert(AUTHORIZATION, HeaderValue::from_static("bearer token"));
        assert_eq!(bearer_token(&headers), Some("token"));

        headers.insert(AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(bearer_token(&headers), None);

        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer   "));
        assert_eq!(bearer_token(&headers), None);

        headers.remove(AUTHORIZATION);
        assert_eq!(bearer_token(&headers), None);
    }
}
