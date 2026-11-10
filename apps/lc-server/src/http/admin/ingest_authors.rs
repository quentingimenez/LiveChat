//! Auteurs d'ingestion et création de clients d'ingestion.
//!
//! Deux routes du contrat legacy :
//!
//! - `GET /admin/api/ingest-authors` : fusion des utilisateurs Discord connus
//!   via les codes de pairing et les clients d'ingestion, avec la même règle de
//!   « dernière apparition » que `listKnownIngestAuthors` du legacy ;
//! - `POST /admin/api/ingest-clients` : crée un client d'ingestion pour un
//!   auteur connu, avec un jeton en clair renvoyé une seule fois (seul le
//!   SHA-256 est stocké, comme `createIngestClientToken`).
//!
//! Le libellé du client est construit comme dans le legacy :
//! `"{label}-{displayName|discordUserId}"`.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use lc_domain::{GuildId, IngestClient, IngestClientId};
use rand::RngCore as _;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

use super::auth::AdminAccess;
use super::{require_db, to_iso, to_non_empty_string, AdminError};
use crate::app::AppState;

/// Origine d'un auteur connu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthorSource {
    Pairing,
    Ingest,
}

/// Auteur agrégé (une entrée par `discordUserId`).
#[derive(Debug, Clone)]
struct AuthorRecord {
    discord_user_id: String,
    display_name: Option<String>,
    image: Option<String>,
    last_seen_at: Option<DateTime<Utc>>,
    seen_via_pairing: bool,
    seen_via_ingest: bool,
}

impl AuthorRecord {
    /// Libellé `source` du contrat (`pairing`, `ingest` ou `mixed`).
    fn source(&self) -> &'static str {
        match (self.seen_via_pairing, self.seen_via_ingest) {
            (true, true) => "mixed",
            (false, true) => "ingest",
            _ => "pairing",
        }
    }

    /// Nom affichable : nom normalisé, sinon identifiant Discord.
    fn display_name(&self) -> &str {
        self.display_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.discord_user_id)
    }
}

/// Handler `GET /admin/api/ingest-authors`.
pub(super) async fn list(
    State(state): State<AppState>,
    _access: AdminAccess,
) -> Result<Response, AdminError> {
    let authors = merged_authors(&state).await?;
    let count = i64::try_from(authors.len()).unwrap_or(0);

    let items: Vec<Value> = authors
        .iter()
        .map(|author| {
            json!({
                "discordUserId": author.discord_user_id,
                "displayName": author.display_name(),
                "image": author.image,
                "lastSeenAt": super::opt_iso(author.last_seen_at),
                "source": author.source(),
            })
        })
        .collect();

    Ok(Json(json!({ "items": items, "count": count })).into_response())
}

/// Handler `POST /admin/api/ingest-clients`.
pub(super) async fn create_client(
    State(state): State<AppState>,
    _access: AdminAccess,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let guild_id = to_non_empty_string(payload.get("guildId"))
        .ok_or_else(|| AdminError::bad_request("invalid_guild_id"))?;
    let guild_id =
        GuildId::parse(&guild_id).map_err(|_| AdminError::bad_request("invalid_guild_id"))?;

    let author_discord_user_id = to_non_empty_string(payload.get("authorDiscordUserId"))
        .ok_or_else(|| AdminError::bad_request("invalid_author_discord_user_id"))?;

    let label = to_non_empty_string(payload.get("label"))
        .ok_or_else(|| AdminError::bad_request("invalid_label"))?;

    let db = require_db(&state)?;
    let known_authors = merged_authors(&state).await?;
    let author = known_authors
        .iter()
        .find(|author| author.discord_user_id == author_discord_user_id)
        .ok_or_else(|| AdminError::bad_request("author_not_found"))?;

    let author_display = author.display_name().to_owned();
    let device_label = format!("{label}-{author_display}");

    let raw_token = generate_token();
    let token_hash = hash_token(&raw_token);
    let now = state.clock.now();

    let client = IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: device_label.clone(),
        token_hash,
        default_author_name: author_display.clone(),
        default_author_image: author.image.clone(),
        created_by_discord_user_id: author.discord_user_id.clone(),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    };

    db.ingest_clients()
        .create(&client)
        .await
        .map_err(|error| AdminError::db("création du client d'ingestion", error))?;

    Ok(Json(json!({
        "created": true,
        "ingestApiToken": raw_token,
        "client": {
            "id": client.id.to_canonical(),
            "guildId": client.guild_id.as_str(),
            "label": client.label,
            "defaultAuthorName": client.default_author_name,
            "defaultAuthorImage": client.default_author_image,
            "createdByDiscordUserId": client.created_by_discord_user_id,
            "createdAt": to_iso(client.created_at),
        },
    }))
    .into_response())
}

/// Fusionne les auteurs connus (pairing + ingestion) par `discordUserId`.
///
/// La liste de pairing d'abord, puis les clients d'ingestion, chacun trié par
/// `created_at` décroissant (ordre du legacy). La première apparition pose le
/// nom/image ; une apparition plus récente les remplace ; une apparition plus
/// ancienne ne complète que les champs manquants.
async fn merged_authors(state: &AppState) -> Result<Vec<AuthorRecord>, AdminError> {
    let db = require_db(state)?;

    let mut pairing_codes = db
        .pairing_codes()
        .list()
        .await
        .map_err(|error| AdminError::db("codes de pairing", error))?;
    pairing_codes.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));

    let mut ingest_clients = db
        .ingest_clients()
        .list()
        .await
        .map_err(|error| AdminError::db("clients d'ingestion", error))?;
    ingest_clients.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));

    let mut authors: BTreeMap<String, AuthorRecord> = BTreeMap::new();

    for code in pairing_codes {
        upsert_author(
            &mut authors,
            &code.created_by_discord_user_id,
            clean(code.author_name),
            clean(code.author_image),
            code.created_at,
            AuthorSource::Pairing,
        );
    }

    for client in ingest_clients {
        upsert_author(
            &mut authors,
            &client.created_by_discord_user_id,
            clean(Some(client.default_author_name.clone())),
            clean(client.default_author_image),
            client.created_at,
            AuthorSource::Ingest,
        );
    }

    let mut authors: Vec<AuthorRecord> = authors.into_values().collect();
    authors.sort_by(|left, right| {
        left.display_name()
            .to_lowercase()
            .cmp(&right.display_name().to_lowercase())
            .then_with(|| left.discord_user_id.cmp(&right.discord_user_id))
    });

    Ok(authors)
}

/// Applique la règle d'upsert du legacy à une entrée du registre.
fn upsert_author(
    authors: &mut BTreeMap<String, AuthorRecord>,
    discord_user_id: &str,
    display_name: Option<String>,
    image: Option<String>,
    created_at: DateTime<Utc>,
    source: AuthorSource,
) {
    let Some(existing) = authors.get_mut(discord_user_id) else {
        authors.insert(
            discord_user_id.to_owned(),
            AuthorRecord {
                discord_user_id: discord_user_id.to_owned(),
                display_name,
                image,
                last_seen_at: Some(created_at),
                seen_via_pairing: source == AuthorSource::Pairing,
                seen_via_ingest: source == AuthorSource::Ingest,
            },
        );
        return;
    };

    existing.seen_via_pairing |= source == AuthorSource::Pairing;
    existing.seen_via_ingest |= source == AuthorSource::Ingest;

    let is_newer = existing
        .last_seen_at
        .is_none_or(|current| created_at > current);

    if is_newer {
        existing.last_seen_at = Some(created_at);
        if display_name.is_some() {
            existing.display_name = display_name;
        }
        if image.is_some() {
            existing.image = image;
        }
    } else {
        let display_missing = existing
            .display_name
            .as_deref()
            .is_none_or(|name| name.is_empty() || name == discord_user_id);
        if display_missing && display_name.is_some() {
            existing.display_name = display_name;
        }
        if existing.image.is_none() && image.is_some() {
            existing.image = image;
        }
    }
}

/// Chaîne trimée non vide, ou `None` (normalisation des noms legacy).
fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Jeton opaque de 32 octets aléatoires, encodé en hexadécimal (64 caractères).
fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 hexadécimal du jeton (seule forme stockée en base).
fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use chrono::TimeZone as _;

    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).single().expect("date valide")
    }

    #[test]
    fn fusion_garde_les_sources_et_la_date_la_plus_recente() {
        let mut authors = BTreeMap::new();
        upsert_author(
            &mut authors,
            "42",
            Some("Alice".to_owned()),
            None,
            at(100),
            AuthorSource::Pairing,
        );
        upsert_author(
            &mut authors,
            "42",
            Some("Alice B".to_owned()),
            Some("https://img".to_owned()),
            at(200),
            AuthorSource::Ingest,
        );

        let record = authors.get("42").expect("auteur présent");
        assert_eq!(record.display_name(), "Alice B");
        assert_eq!(record.image.as_deref(), Some("https://img"));
        assert_eq!(record.last_seen_at, Some(at(200)));
        assert_eq!(record.source(), "mixed");
    }

    #[test]
    fn fusion_complete_les_champs_manquants_sans_ecraser_les_recents() {
        let mut authors = BTreeMap::new();
        upsert_author(
            &mut authors,
            "7",
            Some("Bob".to_owned()),
            None,
            at(300),
            AuthorSource::Pairing,
        );
        upsert_author(
            &mut authors,
            "7",
            Some("Bob ancien".to_owned()),
            Some("https://old".to_owned()),
            at(100),
            AuthorSource::Ingest,
        );

        let record = authors.get("7").expect("auteur présent");
        assert_eq!(record.display_name(), "Bob");
        assert_eq!(record.image.as_deref(), Some("https://old"));
        assert_eq!(record.last_seen_at, Some(at(300)));
    }

    #[test]
    fn nom_absent_retombe_sur_lidentifiant() {
        let mut authors = BTreeMap::new();
        upsert_author(&mut authors, "99", None, None, at(1), AuthorSource::Ingest);

        let record = authors.get("99").expect("auteur présent");
        assert_eq!(record.display_name(), "99");
        assert_eq!(record.source(), "ingest");
    }

    #[test]
    fn jeton_aleatoire_et_hachage_stable() {
        let first = generate_token();
        let second = generate_token();

        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(first, second);
        assert_eq!(hash_token("abc").len(), 64);
        assert_eq!(hash_token("abc"), hash_token("abc"));
        assert_ne!(hash_token("abc"), hash_token("abd"));
    }
}
