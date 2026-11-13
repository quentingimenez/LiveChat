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
use axum::http::HeaderMap;
use lc_db::Db;
use lc_domain::entities::IngestClient;

use crate::app::AppState;
use crate::media::errors::MediaServiceResult;
use crate::media::tokens::hash_ingest_token;

/// Résultat de la résolution d'authentification.
#[derive(Debug)]
pub enum IngestAuth {
    /// Requête authentifiée par un client d'ingestion actif.
    Client(IngestClient),
    /// Aucune authentification valide ; `enabled` indique si l'ingestion est
    /// configurée (au moins un client actif), pour choisir entre 401 et 503.
    Missing { enabled: bool },
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
            // Heartbeat applicatif : une erreur d'écriture ne doit pas refuser
            // une requête authentifiée valide.
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
        None => Ok(IngestAuth::Missing {
            enabled: client_is_enabled(db).await?,
        }),
    }
}

/// `true` si au moins un client d'ingestion non révoqué existe.
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
