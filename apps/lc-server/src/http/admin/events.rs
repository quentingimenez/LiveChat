//! Live admin event stream (`GET /admin/api/events`, section 8).
//!
//! The stream is fed by [`AdminEvents`], a broadcast channel carried by
//! [`AppState`]. Producers only signal *that* something changed:
//!
//! - [`AdminEvent::Overview`] is coalesced by each connection and emitted at
//!   most once per second (plus once immediately when the connection opens);
//! - [`AdminEvent::Audit`] and [`AdminEvent::Ingestion`] carry the freshly
//!   added entry and are forwarded as soon as they arrive.
//!
//! Overview payloads are built by [`super::overview::build_overview`], the same
//! function behind `GET /admin/api/overview`, so the two representations cannot
//! drift. The response carries `Cache-Control: no-store` and the global gzip
//! layer explicitly skips `text/event-stream` (see `app::compression_predicate`).
//!
//! Authentication is the same as every `/admin/api/**` route: the
//! [`AdminAccess`] extractor replies `401` without a session.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::http::{header, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::Router;
use futures_util::stream::{self, Stream};
use lc_domain::GuildId;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{sleep_until, Instant};

use super::auth::AdminAccess;
use super::overview::build_overview;
use super::AdminError;
use crate::app::AppState;

/// Minimum delay between two `overview` events on one connection.
const OVERVIEW_MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Keep-alive interval of the SSE connection.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Capacity of the broadcast channel. When a slow client lags, it receives
/// [`RecvError::Lagged`] and simply refreshes its overview.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// One admin event.
#[derive(Debug, Clone)]
pub enum AdminEvent {
    /// Something changed in the overview (guild, overlay, queue, board, setting).
    Overview,
    /// One entry was appended to the admin audit log.
    Audit(Value),
    /// One entry was appended to the ingestion history.
    Ingestion(Value),
}

/// Broadcast channel shared by the admin middleware, the WebSocket hub and the
/// ingestion registry.
#[derive(Debug, Clone)]
pub struct AdminEvents {
    sender: broadcast::Sender<AdminEvent>,
}

impl Default for AdminEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl AdminEvents {
    /// Creates the channel (no subscriber until the first SSE connection).
    #[must_use]
    pub fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self { sender }
    }

    /// Subscribes to the event stream.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<AdminEvent> {
        self.sender.subscribe()
    }

    /// Signals an overview change; coalesced by each connection.
    pub fn overview_changed(&self) {
        self.send(AdminEvent::Overview);
    }

    /// Publishes one audit entry.
    pub fn audit(&self, entry: Value) {
        self.send(AdminEvent::Audit(entry));
    }

    /// Publishes one ingestion entry.
    pub fn ingestion(&self, entry: Value) {
        self.send(AdminEvent::Ingestion(entry));
    }

    /// Sends an event, ignoring "no subscriber" (SSE is optional).
    fn send(&self, event: AdminEvent) {
        if self.sender.send(event).is_err() {
            tracing::trace!("admin event dropped: no SSE subscriber");
        }
    }
}

/// Sub-router of the live event stream.
pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/admin/api/events", get(events))
}

/// `GET /admin/api/events`.
pub(super) async fn events(
    State(state): State<AppState>,
    _access: AdminAccess,
) -> Result<Response, AdminError> {
    // Subscribe before building the first payload so a change happening during
    // the database round-trip is not lost.
    let receiver = state.events.subscribe();
    let initial = build_overview(&state).await?;
    let stream = sse_stream(state, receiver, initial);

    let sse = Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(KEEP_ALIVE_INTERVAL)
            .text("keep-alive"),
    );

    // The admin middleware would also add `no-store`; setting it here keeps the
    // contract explicit if the stream is served by another router in a test.
    Ok((
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        sse,
    )
        .into_response())
}

/// Mutable state of one SSE connection.
struct StreamState {
    state: AppState,
    receiver: broadcast::Receiver<AdminEvent>,
    /// Overview payload to emit first (the one captured at connection time).
    initial: Option<Value>,
    /// Events received while an overview refresh is pending. They are emitted
    /// after the overview so a mutation never hides the refresh behind its own
    /// audit row.
    pending: VecDeque<Event>,
    /// Instant of the last emitted overview, used by the debounce.
    last_overview: Instant,
    /// `true` when at least one overview change arrived since the last emit.
    dirty: bool,
}

/// Completes an `ingestion` event with the fields the registry cannot know.
///
/// The registry only has the guild ID; the console shows its name, resolved
/// here when a database is configured. A missing database or unknown guild
/// leaves the payload untouched: forwarding an event never fails on this
/// lookup.
async fn enrich_ingestion(state: &AppState, mut entry: Value) -> Value {
    let Some(db) = state.db.as_ref() else {
        return entry;
    };
    let Some(guild_id) = entry
        .get("guildId")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return entry;
    };
    let Ok(guild_id) = GuildId::parse(&guild_id) else {
        return entry;
    };
    match db.guilds().get(&guild_id).await {
        Ok(Some(guild)) => {
            if let Some(object) = entry.as_object_mut() {
                object.insert(
                    "guildName".to_owned(),
                    guild.name.map_or(Value::Null, Value::String),
                );
            }
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "admin SSE guild name lookup failed"),
    }
    entry
}

/// Builds the SSE stream.
fn sse_stream(
    state: AppState,
    receiver: broadcast::Receiver<AdminEvent>,
    initial: Value,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let initial_state = StreamState {
        state,
        receiver,
        initial: Some(initial),
        pending: VecDeque::new(),
        last_overview: Instant::now(),
        dirty: false,
    };

    stream::unfold(initial_state, |mut stream_state| async move {
        loop {
            if let Some(payload) = stream_state.initial.take() {
                return Some((Ok(overview_event(payload)), stream_state));
            }

            if let Some(event) = stream_state.pending.pop_front() {
                return Some((Ok(event), stream_state));
            }

            if stream_state.dirty
                && Instant::now() >= stream_state.last_overview + OVERVIEW_MIN_INTERVAL
            {
                match build_overview(&stream_state.state).await {
                    Ok(payload) => {
                        stream_state.dirty = false;
                        stream_state.last_overview = Instant::now();
                        return Some((Ok(overview_event(payload)), stream_state));
                    }
                    Err(error) => {
                        tracing::warn!(?error, "admin SSE overview refresh failed");
                        stream_state.dirty = false;
                    }
                }
            }

            if stream_state.dirty {
                let deadline = stream_state.last_overview + OVERVIEW_MIN_INTERVAL;
                tokio::select! {
                    received = stream_state.receiver.recv() => {
                        match received {
                            Ok(AdminEvent::Overview) => stream_state.dirty = true,
                            Ok(AdminEvent::Audit(entry)) => {
                                stream_state.pending.push_back(named_event("audit", entry));
                            }
                            Ok(AdminEvent::Ingestion(entry)) => {
                                let entry = enrich_ingestion(&stream_state.state, entry).await;
                                stream_state.pending.push_back(named_event("ingestion", entry));
                            }
                            Err(RecvError::Lagged(count)) => {
                                tracing::debug!(count, "admin SSE client lagged; refreshing overview");
                                stream_state.dirty = true;
                            }
                            Err(RecvError::Closed) => return None,
                        }
                    }
                    () = sleep_until(deadline) => {}
                }
            } else {
                match stream_state.receiver.recv().await {
                    Ok(AdminEvent::Overview) => stream_state.dirty = true,
                    Ok(AdminEvent::Audit(entry)) => {
                        return Some((Ok(named_event("audit", entry)), stream_state));
                    }
                    Ok(AdminEvent::Ingestion(entry)) => {
                        let entry = enrich_ingestion(&stream_state.state, entry).await;
                        return Some((Ok(named_event("ingestion", entry)), stream_state));
                    }
                    Err(RecvError::Lagged(count)) => {
                        tracing::debug!(count, "admin SSE client lagged; refreshing overview");
                        stream_state.dirty = true;
                    }
                    Err(RecvError::Closed) => return None,
                }
            }
        }
    })
}

/// Builds an `overview` SSE event.
fn overview_event(payload: Value) -> Event {
    named_event("overview", payload)
}

/// Builds a named SSE event with a JSON payload.
///
/// A [`Value`] always serialises; the empty object fallback only exists to keep
/// the function panic-free.
fn named_event(name: &'static str, payload: Value) -> Event {
    match Event::default().event(name).json_data(payload) {
        Ok(event) => event,
        Err(error) => {
            tracing::error!(%error, event = name, "admin SSE payload serialisation failed");
            Event::default().event(name).data("{}")
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[tokio::test]
    async fn overview_signals_are_broadcast() {
        let events = AdminEvents::new();
        let mut receiver = events.subscribe();
        events.overview_changed();
        assert!(matches!(
            receiver.recv().await.expect("event"),
            AdminEvent::Overview
        ));
    }
}
