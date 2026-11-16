//! In-memory registry of asynchronous media-board ingestions.
//!
//! Each `POST /overlay/media-board/items` or `/upload` returns `202
//! {ingestionId}` before the download/transcode runs in a background task. The
//! registry is the shared source of truth for those tasks:
//!
//! - it caps the number of concurrent `processing` ingestions per overlay
//!   client ([`MAX_PROCESSING_PER_CLIENT`]) so a client cannot queue unbounded
//!   work;
//! - it keeps finished entries for [`FINISHED_RETENTION`] so a board that
//!   reconnects can fetch a result it missed through
//!   `GET /overlay/media-board/ingestions`;
//! - it is deliberately tiny and synchronous: the mutex is never held across an
//!   `.await` and every caller passes the current instant from
//!   [`crate::app::AppState::clock`] (deterministic tests).
//!
//! Entries are purged lazily on insert and read; no background timer is needed.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Duration, Utc};
use lc_domain::GuildId;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::http::admin::AdminEvents;

/// Maximum number of `processing` ingestions per overlay client.
pub const MAX_PROCESSING_PER_CLIENT: usize = 20;

/// How long a finished ingestion stays available to the board.
pub const FINISHED_RETENTION: Duration = Duration::hours(1);

/// Number of finished ingestions kept in the admin history.
pub const HISTORY_LIMIT: usize = 200;

/// Lifecycle of an ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestionStatus {
    /// The background task is still downloading/transcoding.
    Processing,
    /// The media is on the board (`created` tells whether it was new).
    Ready,
    /// The ingestion failed; `error` carries the reason.
    Failed,
}

impl IngestionStatus {
    /// Wire name used by `GET /overlay/media-board/ingestions`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Processing => "processing",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

/// Failure detail stored with a failed ingestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestionErrorInfo {
    /// Machine code (`media_ingestion_failed`, `unsupported_media`,
    /// `internal_error`, ...).
    pub code: String,
    /// Readable detail, when available.
    pub message: Option<String>,
}

/// Snapshot of one ingestion.
#[derive(Debug, Clone)]
pub struct IngestionEntry {
    /// Identifier returned by the `202` response.
    pub ingestion_id: String,
    /// Guild of the authenticated overlay client.
    pub guild_id: GuildId,
    /// UUID of the requesting overlay client (target of `media.ingestion`).
    pub client_id: Uuid,
    /// Current lifecycle state.
    pub status: IngestionStatus,
    /// Instant the ingestion was accepted.
    pub created_at: DateTime<Utc>,
    /// `true` when a new board item was created, `false` when the media was
    /// already pinned; only set once the ingestion is `ready`.
    pub created: Option<bool>,
    /// Board item holding the media; only set once the ingestion is `ready`.
    pub item_id: Option<String>,
    /// Why the ingestion failed; only set once the ingestion is `failed`.
    pub error: Option<IngestionErrorInfo>,
    /// Instant the background task finished (success or failure).
    pub finished_at: Option<DateTime<Utc>>,
}

impl IngestionEntry {
    /// JSON shape returned by `GET /overlay/media-board/ingestions`.
    ///
    /// Optional fields are omitted instead of serialized as `null`, matching
    /// the protocol's `media.ingestion` payload.
    #[must_use]
    pub fn payload(&self) -> Value {
        let mut object = Map::new();
        object.insert(
            "ingestionId".to_owned(),
            Value::String(self.ingestion_id.clone()),
        );
        object.insert(
            "status".to_owned(),
            Value::String(self.status.as_str().to_owned()),
        );
        if let Some(created) = self.created {
            object.insert("created".to_owned(), Value::Bool(created));
        }
        if let Some(item_id) = &self.item_id {
            object.insert("itemId".to_owned(), Value::String(item_id.clone()));
        }
        if let Some(error) = &self.error {
            let mut error_object = Map::new();
            error_object.insert("code".to_owned(), Value::String(error.code.clone()));
            if let Some(message) = &error.message {
                error_object.insert("message".to_owned(), Value::String(message.clone()));
            }
            object.insert("error".to_owned(), Value::Object(error_object));
        }
        Value::Object(object)
    }
}

/// One finished ingestion, retained for the admin console.
///
/// Unlike [`IngestionEntry`], this history outlives [`FINISHED_RETENTION`] and
/// is bounded by [`HISTORY_LIMIT`]. It deliberately stores no model payload:
/// the admin handler resolves the guild name and the source URL from SQLite.
#[derive(Debug, Clone)]
pub struct FinishedIngestion {
    /// Identifier returned by the `202` response.
    pub ingestion_id: String,
    /// Guild of the authenticated overlay client.
    pub guild_id: GuildId,
    /// Board item holding the media, when the ingestion succeeded.
    pub item_id: Option<String>,
    /// Terminal lifecycle state (`Ready` or `Failed`).
    pub status: IngestionStatus,
    /// Why the ingestion failed, if it did.
    pub error: Option<IngestionErrorInfo>,
    /// Instant the ingestion was accepted.
    pub started_at: DateTime<Utc>,
    /// Instant the background task finished.
    pub finished_at: DateTime<Utc>,
}

/// Why a new ingestion cannot start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartIngestionError {
    /// The client already has [`MAX_PROCESSING_PER_CLIENT`] ingestions running.
    TooManyProcessing,
}

/// Shared in-memory ingestion registry.
#[derive(Debug, Default)]
pub struct IngestionRegistry {
    entries: Mutex<HashMap<String, IngestionEntry>>,
    /// Bounded history of finished ingestions, oldest first.
    history: Mutex<VecDeque<FinishedIngestion>>,
    /// Optional admin event channel (`ingestion` SSE events).
    events: Option<AdminEvents>,
}

impl IngestionRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty registry publishing `ingestion` events.
    #[must_use]
    pub fn with_events(events: AdminEvents) -> Self {
        Self {
            events: Some(events),
            ..Self::default()
        }
    }

    /// Locks the registry, tolerating a poisoned mutex.
    fn lock(&self) -> MutexGuard<'_, HashMap<String, IngestionEntry>> {
        match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Locks the finished history, tolerating a poisoned mutex.
    fn lock_history(&self) -> MutexGuard<'_, VecDeque<FinishedIngestion>> {
        match self.history.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Registers a new `processing` ingestion and returns its UUID v4.
    ///
    /// Fails with [`StartIngestionError::TooManyProcessing`] once the client has
    /// [`MAX_PROCESSING_PER_CLIENT`] running ingestions. Finished entries older
    /// than [`FINISHED_RETENTION`] are purged before the check.
    pub fn start(
        &self,
        guild_id: GuildId,
        client_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<String, StartIngestionError> {
        let ingestion_id = Uuid::new_v4().to_string();
        let mut entries = self.lock();
        purge_finished(&mut entries, now);
        let processing = entries
            .values()
            .filter(|entry| {
                entry.client_id == client_id && entry.status == IngestionStatus::Processing
            })
            .count();
        if processing >= MAX_PROCESSING_PER_CLIENT {
            return Err(StartIngestionError::TooManyProcessing);
        }
        entries.insert(
            ingestion_id.clone(),
            IngestionEntry {
                ingestion_id: ingestion_id.clone(),
                guild_id,
                client_id,
                status: IngestionStatus::Processing,
                created_at: now,
                created: None,
                item_id: None,
                error: None,
                finished_at: None,
            },
        );
        Ok(ingestion_id)
    }

    /// Marks an ingestion as `ready` with its board outcome.
    pub fn complete_success(
        &self,
        ingestion_id: &str,
        created: bool,
        item_id: String,
        now: DateTime<Utc>,
    ) {
        let mut entries = self.lock();
        purge_finished(&mut entries, now);
        let finished = {
            let Some(entry) = entries.get_mut(ingestion_id) else {
                return;
            };
            let already_finished = entry.finished_at.is_some();
            entry.status = IngestionStatus::Ready;
            entry.created = Some(created);
            entry.item_id = Some(item_id.clone());
            entry.error = None;
            entry.finished_at = Some(now);
            (!already_finished).then(|| FinishedIngestion {
                ingestion_id: entry.ingestion_id.clone(),
                guild_id: entry.guild_id.clone(),
                item_id: Some(item_id),
                status: IngestionStatus::Ready,
                error: None,
                started_at: entry.created_at,
                finished_at: now,
            })
        };
        drop(entries);
        if let Some(finished) = finished {
            self.push_history(finished);
        }
    }

    /// Marks an ingestion as `failed` with a readable error.
    pub fn complete_failure(
        &self,
        ingestion_id: &str,
        error: IngestionErrorInfo,
        now: DateTime<Utc>,
    ) {
        let mut entries = self.lock();
        purge_finished(&mut entries, now);
        let finished = {
            let Some(entry) = entries.get_mut(ingestion_id) else {
                return;
            };
            let already_finished = entry.finished_at.is_some();
            entry.status = IngestionStatus::Failed;
            entry.created = None;
            entry.item_id = None;
            entry.error = Some(error.clone());
            entry.finished_at = Some(now);
            (!already_finished).then(|| FinishedIngestion {
                ingestion_id: entry.ingestion_id.clone(),
                guild_id: entry.guild_id.clone(),
                item_id: None,
                status: IngestionStatus::Failed,
                error: Some(error),
                started_at: entry.created_at,
                finished_at: now,
            })
        };
        drop(entries);
        if let Some(finished) = finished {
            self.push_history(finished);
        }
    }

    /// Appends a finished ingestion, keeping at most [`HISTORY_LIMIT`] rows.
    fn push_history(&self, finished: FinishedIngestion) {
        if let Some(events) = self.events.as_ref() {
            events.ingestion(ingestion_event(&finished));
        }
        let mut history = self.lock_history();
        if history.len() >= HISTORY_LIMIT {
            history.pop_front();
        }
        history.push_back(finished);
    }

    /// Returns the finished ingestion history, oldest first.
    #[must_use]
    pub fn finished(&self) -> Vec<FinishedIngestion> {
        self.lock_history().iter().cloned().collect()
    }

    /// Returns the ingestions still being processed, newest first.
    #[must_use]
    pub fn active(&self, now: DateTime<Utc>) -> Vec<IngestionEntry> {
        let mut entries = self.lock();
        purge_finished(&mut entries, now);
        let mut active: Vec<IngestionEntry> = entries
            .values()
            .filter(|entry| entry.status == IngestionStatus::Processing)
            .cloned()
            .collect();
        active.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.ingestion_id.cmp(&left.ingestion_id))
        });
        active
    }

    /// Counts failures finished at or after `since`.
    #[must_use]
    pub fn failure_count_since(&self, since: DateTime<Utc>) -> usize {
        self.lock_history()
            .iter()
            .filter(|entry| entry.status == IngestionStatus::Failed && entry.finished_at >= since)
            .count()
    }

    /// Returns one ingestion owned by `client_id`, if it is still retained.
    ///
    /// Ingestions of other clients are never exposed.
    #[must_use]
    pub fn get(
        &self,
        client_id: Uuid,
        ingestion_id: &str,
        now: DateTime<Utc>,
    ) -> Option<IngestionEntry> {
        let mut entries = self.lock();
        purge_finished(&mut entries, now);
        entries
            .get(ingestion_id)
            .filter(|entry| entry.client_id == client_id)
            .cloned()
    }

    /// Lists the retained ingestions of one overlay client, newest first.
    #[must_use]
    pub fn list_for_client(&self, client_id: Uuid, now: DateTime<Utc>) -> Vec<IngestionEntry> {
        let mut entries = self.lock();
        purge_finished(&mut entries, now);
        let mut listed: Vec<IngestionEntry> = entries
            .values()
            .filter(|entry| entry.client_id == client_id)
            .cloned()
            .collect();
        listed.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.ingestion_id.cmp(&left.ingestion_id))
        });
        listed
    }
}

/// JSON payload of one `ingestion` admin event.
///
/// It follows the shape of an item of `GET /admin/api/ingestions`; the guild
/// name and the source URL require database lookups and are therefore left
/// empty (`guildName: null`, `source: "unknown"`).
fn ingestion_event(finished: &FinishedIngestion) -> Value {
    let status = match finished.status {
        IngestionStatus::Ready => "done",
        IngestionStatus::Failed => "failed",
        IngestionStatus::Processing => "running",
    };
    let error = finished
        .error
        .as_ref()
        .map(|error| error.message.clone().unwrap_or_else(|| error.code.clone()));
    let duration_ms = finished
        .finished_at
        .signed_duration_since(finished.started_at)
        .num_milliseconds()
        .max(0);

    json!({
        "id": finished.ingestion_id,
        "guildId": finished.guild_id.as_str(),
        "guildName": Value::Null,
        "source": "unknown",
        "url": "",
        "status": status,
        "error": error,
        "startedAt": finished
            .started_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "finishedAt": finished
            .finished_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "durationMs": duration_ms,
    })
}

/// Drops finished entries whose retention window elapsed.
fn purge_finished(entries: &mut HashMap<String, IngestionEntry>, now: DateTime<Utc>) {
    entries.retain(|_, entry| {
        entry
            .finished_at
            .is_none_or(|finished_at| finished_at + FINISHED_RETENTION > now)
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use chrono::TimeZone as _;

    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
            .single()
            .unwrap()
    }

    fn guild() -> GuildId {
        GuildId::parse("123456789012345678").unwrap()
    }

    #[test]
    fn start_limits_processing_entries_per_client() {
        let registry = IngestionRegistry::new();
        let client = Uuid::new_v4();
        for _ in 0..MAX_PROCESSING_PER_CLIENT {
            registry.start(guild(), client, now()).unwrap();
        }
        assert_eq!(
            registry.start(guild(), client, now()).unwrap_err(),
            StartIngestionError::TooManyProcessing
        );
        // Another client is unaffected.
        registry.start(guild(), Uuid::new_v4(), now()).unwrap();
    }

    #[test]
    fn finished_entries_are_retained_for_one_hour_then_purged() {
        let registry = IngestionRegistry::new();
        let client = Uuid::new_v4();
        let id = registry.start(guild(), client, now()).unwrap();
        registry.complete_success(&id, true, "item-1".to_owned(), now());
        assert!(registry.get(client, &id, now()).is_some());

        let later = now() + Duration::minutes(59);
        assert!(registry.get(client, &id, later).is_some());

        let too_late = now() + Duration::minutes(61);
        assert!(registry.get(client, &id, too_late).is_none());
        assert!(registry.list_for_client(client, too_late).is_empty());
    }

    #[test]
    fn get_and_list_only_expose_the_requesting_client() {
        let registry = IngestionRegistry::new();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let first_id = registry.start(guild(), first, now()).unwrap();
        let second_id = registry.start(guild(), second, now()).unwrap();

        let listed = registry.list_for_client(first, now());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].ingestion_id, first_id);
        assert!(registry.get(first, &second_id, now()).is_none());
        assert!(registry.get(second, &second_id, now()).is_some());
    }

    #[test]
    fn history_keeps_the_last_two_hundred_finished_ingestions() {
        let registry = IngestionRegistry::new();
        let client = Uuid::new_v4();
        let mut ids = Vec::new();
        for index in 0..(HISTORY_LIMIT + 5) {
            let started = now() + Duration::seconds(i64::try_from(index).unwrap_or(0));
            let id = registry.start(guild(), client, started).unwrap();
            registry.complete_failure(
                &id,
                IngestionErrorInfo {
                    code: "internal_error".to_owned(),
                    message: None,
                },
                started + Duration::seconds(1),
            );
            ids.push(id);
        }

        let history = registry.finished();
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(
            history[0].ingestion_id, ids[5],
            "oldest entries are dropped"
        );
        assert_eq!(
            history[HISTORY_LIMIT - 1].ingestion_id,
            ids[HISTORY_LIMIT + 4]
        );
    }

    #[tokio::test]
    async fn finished_ingestions_are_published_as_admin_events() {
        use crate::http::admin::AdminEvent;

        let events = AdminEvents::new();
        let mut receiver = events.subscribe();
        let registry = IngestionRegistry::with_events(events);
        let client = Uuid::new_v4();
        let id = registry.start(guild(), client, now()).unwrap();
        registry.complete_failure(
            &id,
            IngestionErrorInfo {
                code: "unsupported_media".to_owned(),
                message: Some("source non supportée".to_owned()),
            },
            now(),
        );

        match receiver.recv().await.expect("ingestion event") {
            AdminEvent::Ingestion(entry) => {
                assert_eq!(entry["id"], id);
                assert_eq!(entry["guildId"], "123456789012345678");
                assert_eq!(entry["status"], "failed");
                assert_eq!(entry["error"], "source non supportée");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn failure_payload_carries_code_and_optional_message() {
        let registry = IngestionRegistry::new();
        let client = Uuid::new_v4();
        let id = registry.start(guild(), client, now()).unwrap();
        registry.complete_failure(
            &id,
            IngestionErrorInfo {
                code: "unsupported_media".to_owned(),
                message: None,
            },
            now(),
        );
        let entry = registry.get(client, &id, now()).unwrap();
        assert_eq!(entry.status, IngestionStatus::Failed);
        assert_eq!(
            entry.payload(),
            json!({
                "ingestionId": id,
                "status": "failed",
                "error": { "code": "unsupported_media" },
            })
        );
    }
}
