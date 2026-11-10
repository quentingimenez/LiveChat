//! `GET /admin/api/overview` : agrégats de toutes les guildes pour le panneau.
//!
//! Reproduit `GET /api/overview` du legacy
//! (`server/src/components/admin/adminRoutes.ts:2333-2565`) : mêmes champs,
//! mêmes valeurs calculées, mêmes règles de dégradation (guilde inconnue,
//! réglages absents, compteurs de playback). Différences assumées et
//! documentées :
//!
//! - le nom Discord n'est pas résolu (le serveur Rust n'a pas encore
//!   d'annuaire Discord dans `AppState`) : on utilise le nom persisté, sinon
//!   `Guild {id}` — `editableName` vaut alors `""` comme dans le legacy quand
//!   aucun nom n'est connu ;
//! - les sockets connectées viennent du trait [`super::hub::AdminHub`], pas
//!   d'un `io.fetchSockets()` direct.

use std::collections::{BTreeSet, HashMap};

use axum::extract::State;
use axum::response::{IntoResponse as _, Response};
use axum::Json;
use lc_domain::{Guild, GuildId, OverlaySessionMode};
use serde_json::{json, Value};
use uuid::Uuid;

use super::auth::AdminAccess;
use super::hub::admin_hub;
use super::queries::{self, OverlayClientRow};
use super::{iso_from_ms, megabytes_to_bytes, percent, require_db, to_iso, AdminError};
use crate::app::AppState;

/// Compteurs de playback d'une guilde (statuts terminaux agrégés).
#[derive(Debug, Default, Clone, Copy)]
struct PlaybackTotals {
    pending: i64,
    playing: i64,
    failed: i64,
    done: i64,
}

impl PlaybackTotals {
    fn add(&mut self, status: &str, count: i64) {
        match status {
            "PENDING" => self.pending += count,
            "PLAYING" => self.playing += count,
            "FAILED" => self.failed += count,
            "DONE" => self.done += count,
            _ => {}
        }
    }
}

/// Agrégats d'une guilde pour l'aperçu.
#[derive(Debug, Default, Clone)]
struct GuildAggregates {
    board_items: i64,
    board_bytes: i64,
    playback: PlaybackTotals,
}

/// Handler de l'aperçu.
pub(super) async fn overview(
    State(state): State<AppState>,
    _access: AdminAccess,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let now = state.clock.now();

    let guilds = db
        .guilds()
        .list()
        .await
        .map_err(|error| AdminError::db("guildes", error))?;
    let overlays = queries::overlay_clients(db)
        .await
        .map_err(|error| AdminError::db("clients overlay", error))?;
    let mut ingest_clients = db
        .ingest_clients()
        .list()
        .await
        .map_err(|error| AdminError::db("clients d'ingestion", error))?;
    // Le legacy ne liste que les clients non révoqués.
    ingest_clients.retain(|client| client.is_active());
    ingest_clients.sort_by(|left, right| {
        left.guild_id
            .as_str()
            .cmp(right.guild_id.as_str())
            .then_with(|| left.label.cmp(&right.label))
    });
    let pairing_codes = db
        .pairing_codes()
        .list()
        .await
        .map_err(|error| AdminError::db("codes de pairing", error))?;
    let board_sizes = queries::meme_board_sizes(db)
        .await
        .map_err(|error| AdminError::db("meme board", error))?;
    let playback_counts = queries::playback_counts(db)
        .await
        .map_err(|error| AdminError::db("playback", error))?;
    let cache_used_bytes = queries::cache_used_bytes(db)
        .await
        .map_err(|error| AdminError::db("cache média", error))?;

    // Union des guildes connues (guilde persistée ou données rattachées).
    let mut guild_ids: BTreeSet<String> = BTreeSet::new();
    for guild in &guilds {
        guild_ids.insert(guild.id.as_str().to_owned());
    }
    for overlay in &overlays {
        guild_ids.insert(overlay.guild_id.clone());
    }
    for client in &ingest_clients {
        guild_ids.insert(client.guild_id.as_str().to_owned());
    }
    for (guild_id, _) in &board_sizes {
        guild_ids.insert(guild_id.clone());
    }
    for (guild_id, _, _) in &playback_counts {
        guild_ids.insert(guild_id.clone());
    }
    for code in &pairing_codes {
        guild_ids.insert(code.guild_id.as_str().to_owned());
    }

    let guild_records: HashMap<&str, &Guild> = guilds
        .iter()
        .map(|guild| (guild.id.as_str(), guild))
        .collect();
    let mut aggregates: HashMap<&str, GuildAggregates> = HashMap::new();
    for (guild_id, size_bytes) in &board_sizes {
        let entry = aggregates.entry(guild_id.as_str()).or_default();
        entry.board_items += 1;
        entry.board_bytes += *size_bytes;
    }
    for (guild_id, status, count) in &playback_counts {
        let entry = aggregates.entry(guild_id.as_str()).or_default();
        entry.playback.add(status, *count);
    }

    let cache_max_bytes = megabytes_to_bytes(state.config.media_cache_max_total_mb);
    let board_max_bytes = megabytes_to_bytes(state.config.media_board_max_total_mb);
    let hub = admin_hub(&state);

    let mut guild_payloads: Vec<Value> = Vec::with_capacity(guild_ids.len());
    let mut connected_overlay_count = 0_i64;

    for guild_id in &guild_ids {
        let Ok(parsed_guild_id) = GuildId::parse(guild_id) else {
            tracing::warn!(guild_id, "guilde invalide ignorée dans l'aperçu admin");
            continue;
        };

        let record = guild_records.get(guild_id.as_str()).copied();
        let guild_aggregates = aggregates
            .get(guild_id.as_str())
            .cloned()
            .unwrap_or_else(GuildAggregates::default);

        let persisted_name = record
            .and_then(|guild| guild.name.as_deref())
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let display_name = persisted_name
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Guild {guild_id}"));
        let editable_name = persisted_name.unwrap_or_default();

        let default_media_time = record
            .and_then(|guild| guild.default_media_time)
            .unwrap_or_else(|| i64::try_from(state.config.default_duration).unwrap_or(5));
        let max_media_time = record.and_then(|guild| guild.max_media_time);
        let display_media_full = record
            .map(|guild| guild.display_media_full)
            .unwrap_or(false);
        let settings = json!({
            "defaultMediaTime": default_media_time,
            "maxMediaTime": max_media_time,
            "displayMediaFull": display_media_full,
            "overrides": {
                "defaultMediaTime": record.and_then(|guild| guild.default_media_time),
                "maxMediaTime": max_media_time,
                "displayMediaFull": record.map(|guild| guild.display_media_full),
            },
        });

        let guild_overlays: Vec<&OverlayClientRow> = overlays
            .iter()
            .filter(|overlay| overlay.guild_id == *guild_id)
            .collect();
        let connected_for_guild = hub.connected_for_guild(&parsed_guild_id);
        connected_overlay_count += i64::try_from(connected_for_guild).unwrap_or(0);

        let overlay_clients: Vec<Value> = guild_overlays
            .iter()
            .map(|overlay| {
                let connected = Uuid::parse_str(&overlay.id)
                    .ok()
                    .is_some_and(|client_id| hub.is_connected(&client_id));
                json!({
                    "id": overlay.id,
                    "label": overlay.label,
                    "authorName": overlay.default_author_name,
                    "authorImage": overlay.default_author_image,
                    "lastSeenAt": overlay.last_seen_at.and_then(iso_from_ms),
                    "createdAt": iso_from_ms(overlay.created_at),
                    "connected": connected,
                    "sessionMode": session_mode_wire(&overlay.session_mode),
                })
            })
            .collect();

        let guild_ingest: Vec<&lc_domain::IngestClient> = ingest_clients
            .iter()
            .filter(|client| client.guild_id.as_str() == guild_id)
            .collect();
        let ingest_clients_payload: Vec<Value> = guild_ingest
            .iter()
            .map(|client| {
                json!({
                    "id": client.id.to_canonical(),
                    "label": client.label,
                    "lastSeenAt": opt_iso_ms(client.last_seen_at.map(|date| date.timestamp_millis())),
                    "createdAt": iso_from_ms(client.created_at.timestamp_millis()),
                })
            })
            .collect();

        let used_bytes = guild_aggregates.board_bytes.max(0);
        guild_payloads.push(json!({
            "id": guild_id,
            "name": display_name,
            "editableName": editable_name,
            "busyUntil": opt_iso_ms(record.and_then(|guild| guild.busy_until.map(|date| date.timestamp_millis()))),
            "settings": settings,
            "overlays": {
                "total": overlay_clients.len(),
                "connectedCount": connected_for_guild,
                "clients": overlay_clients,
            },
            "ingest": {
                "total": ingest_clients_payload.len(),
                "clients": ingest_clients_payload,
            },
            "memeBoard": {
                "itemsCount": guild_aggregates.board_items,
                "usedBytes": used_bytes,
                "maxBytes": board_max_bytes,
                "remainingBytes": (board_max_bytes - used_bytes).max(0),
                "usedPercent": percent(used_bytes, board_max_bytes),
            },
            "playback": {
                "pending": guild_aggregates.playback.pending,
                "playing": guild_aggregates.playback.playing,
                "failed": guild_aggregates.playback.failed,
                "done": guild_aggregates.playback.done,
            },
        }));
    }

    let total_pairing = i64::try_from(pairing_codes.len()).unwrap_or(0);
    let active_pairing = pairing_codes
        .iter()
        .filter(|code| code.is_usable_at(now))
        .count();
    let active_pairing = i64::try_from(active_pairing).unwrap_or(0);

    let overlay_total = i64::try_from(overlays.len()).unwrap_or(0);
    let ingest_total = i64::try_from(ingest_clients.len()).unwrap_or(0);
    let guild_total = i64::try_from(guild_payloads.len()).unwrap_or(0);

    let cache_used_bytes = cache_used_bytes.max(0);
    let payload = json!({
        "generatedAt": to_iso(now),
        "cache": {
            "usedBytes": cache_used_bytes,
            "maxBytes": cache_max_bytes,
            "remainingBytes": (cache_max_bytes - cache_used_bytes).max(0),
            "usedPercent": percent(cache_used_bytes, cache_max_bytes),
        },
        "totals": {
            "guildCount": guild_total,
            "overlayClientCount": overlay_total,
            "overlayConnectedCount": connected_overlay_count,
            "ingestClientCount": ingest_total,
            "pairingCodeCount": total_pairing,
        },
        "pairingCodes": {
            "active": active_pairing,
            "expired": total_pairing - active_pairing,
            "total": total_pairing,
        },
        "guilds": guild_payloads,
    });

    Ok(Json(payload).into_response())
}

/// Mode de session au format filaire (`normal` / `invite_read_only`), tolérant
/// aux valeurs inattendues (comme `normalizeOverlaySessionMode`).
fn session_mode_wire(stored: &str) -> &'static str {
    OverlaySessionMode::from_db_str(stored)
        .map(OverlaySessionMode::as_wire_str)
        .unwrap_or("normal")
}

/// Variante de [`super::iso_from_ms`] pour une date déjà convertie.
fn opt_iso_ms(milliseconds: Option<i64>) -> Option<String> {
    milliseconds.and_then(iso_from_ms)
}
