//! `POST /overlay/say`: speech synthesis triggered from the board.
//!
//! Same feature as the Discord commands `/dire` and `/cdire`: the text is
//! synthesized by the local Piper engine ([`crate::media::tts`]), ingested as
//! a normalized audio asset and queued as a playback job for the guild's
//! overlays (`priority` 0, like the Discord commands and `POST /ingest`).
//!
//! Body:
//!
//! | Field | Type | Meaning |
//! |-------|------|---------|
//! | `text` | string, required | text to speak (`voice` option of `/dire`), at most [`MAX_SPEECH_CHARS`] characters once whitespace is collapsed |
//! | `caption` | string, optional | text shown on the overlay while it speaks (`text` option of `/dire`), cut to [`MAX_MEDIA_TEXT_CHARS`] characters |
//! | `anonymous` | boolean, optional | `true` = `/cdire`: the author is not shown |
//!
//! | Case | Status | Body |
//! |------|--------|------|
//! | guest (read-only session) | 403 | `{"error":"read_only_session"}` |
//! | role without `TriggerPlayback` | 403 | `{"error":"forbidden"}` |
//! | malformed JSON, missing or empty `text`, wrong types | 400 | `{"error":"invalid_payload"}` |
//! | `text` too long | 400 | `{"error":"text_too_long","maxChars":1000}` |
//! | no database | 503 | `{"error":"database_unavailable"}` |
//! | Piper or the audio conversion failed | 502 | `{"error":"speech_failed"}` |
//! | success | 200 | `{"queued":true,"jobId","durationSec"}` |
//!
//! Unlike Discord, which silently cuts the text, the route refuses a text
//! over the limit: the board shows a counter and the member keeps control of
//! what is spoken.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use lc_discord::{Lang, SpeechFailure};
use lc_domain::Permission;
use lc_protocol::MAX_MEDIA_TEXT_CHARS;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::media::jobs::{self, JobSpec};
use crate::media::tts::{self, MAX_SPEECH_CHARS};
use crate::media::{errors::MediaServiceError, hook};

use super::auth::AuthedOverlayClient;
use super::guild::require;
use super::media_board::{ensure_writable, json_rejection_error};
use super::{db, db_failure, error, internal_error, json_response, now};

/// `400 invalid_payload` with a short explanation.
fn invalid(message: &'static str) -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        json!({"error": "invalid_payload", "message": message}),
    )
}

/// Validated request.
#[derive(Debug, PartialEq, Eq)]
struct SayRequest {
    /// Text to speak, whitespace collapsed.
    speech: String,
    /// Text shown with it, if any.
    caption: Option<String>,
    /// `/cdire` variant: no author.
    anonymous: bool,
}

/// Why a body is refused.
#[derive(Debug, PartialEq, Eq)]
enum SayRejection {
    Invalid(&'static str),
    TooLong,
}

fn parse_request(body: &Value) -> Result<SayRequest, SayRejection> {
    let Some(object) = body.as_object() else {
        return Err(SayRejection::Invalid("a JSON object is required"));
    };
    let speech = match object.get("text") {
        Some(Value::String(text)) => tts::collapse_speech(text),
        _ => return Err(SayRejection::Invalid("text is required")),
    };
    if speech.is_empty() {
        return Err(SayRejection::Invalid("text is required"));
    }
    if speech.chars().count() > MAX_SPEECH_CHARS {
        return Err(SayRejection::TooLong);
    }
    let caption = match object.get("caption") {
        None | Some(Value::Null) => None,
        Some(Value::String(caption)) => Some(caption.trim())
            .filter(|caption| !caption.is_empty())
            .map(|caption| caption.chars().take(MAX_MEDIA_TEXT_CHARS).collect()),
        Some(_) => return Err(SayRejection::Invalid("caption must be a string")),
    };
    let anonymous = match object.get("anonymous") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(anonymous)) => *anonymous,
        Some(_) => return Err(SayRejection::Invalid("anonymous must be a boolean")),
    };
    Ok(SayRequest {
        speech,
        caption,
        anonymous,
    })
}

/// Author shown with the job: the member's live profile, or the client's
/// default author for an overlay without a member (same rule as a board
/// playback request).
fn author(auth: &AuthedOverlayClient) -> (Option<String>, Option<String>) {
    match &auth.member {
        Some(member) => (Some(member.display_name.clone()), member.avatar.clone()),
        None => (
            auth.client.default_author_name.clone(),
            auth.client.default_author_image.clone(),
        ),
    }
}

/// Handler `POST /overlay/say`.
pub async fn say(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    if let Err(response) = require(&state, &auth, Permission::TriggerPlayback) {
        return response;
    }
    let body = match payload {
        Ok(Json(body)) => body,
        Err(rejection) => return json_rejection_error(rejection),
    };
    let request = match parse_request(&body) {
        Ok(request) => request,
        Err(SayRejection::Invalid(message)) => return invalid(message),
        Err(SayRejection::TooLong) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "text_too_long", "maxChars": MAX_SPEECH_CHARS}),
            );
        }
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let lang = Lang::parse(&state.config.i18n);
    let asset = match tts::ingest_speech(&state, lang, &request.speech).await {
        Ok(asset) => asset,
        Err(SpeechFailure::EmptyText) => return invalid("text is required"),
        // The cause is already logged by the speech module.
        Err(_) => return error(StatusCode::BAD_GATEWAY, "speech_failed"),
    };

    let (author_name, author_image) = if request.anonymous {
        (None, None)
    } else {
        author(&auth)
    };
    let spec = JobSpec {
        guild_id: auth.guild_id().clone(),
        media: Some(&asset),
        text: request.caption.as_deref(),
        show_text: request.caption.is_some(),
        author_name: author_name.as_deref(),
        author_image: author_image.as_deref(),
        duration_sec: None,
        priority: 0,
    };
    match jobs::create_for_ingest(db, state.runtime().default_duration(), spec, now(&state)).await {
        Ok(job) => {
            hook::notify_ingested(&job).await;
            tracing::info!(
                job_id = %job.id.to_canonical(),
                guild_id = job.guild_id.as_str(),
                anonymous = request.anonymous,
                "speech job created from the board"
            );
            json_response(
                StatusCode::OK,
                json!({
                    "queued": true,
                    "jobId": job.id.to_canonical(),
                    "durationSec": job.duration_sec,
                }),
            )
        }
        Err(MediaServiceError::Database(db_error)) => db_failure("overlay_say.job", &db_error),
        Err(service_error) => {
            tracing::error!(error = %service_error, "speech job creation failed");
            internal_error()
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use serde_json::json;

    use super::{parse_request, SayRejection, SayRequest, MAX_MEDIA_TEXT_CHARS, MAX_SPEECH_CHARS};

    #[test]
    fn text_is_collapsed_and_options_default_to_dire() {
        assert_eq!(
            parse_request(&json!({"text": "  bonjour \n tout   le monde "})),
            Ok(SayRequest {
                speech: "bonjour tout le monde".to_owned(),
                caption: None,
                anonymous: false,
            })
        );
    }

    #[test]
    fn caption_and_anonymous_are_read() {
        assert_eq!(
            parse_request(&json!({"text": "salut", "caption": "  coucou ", "anonymous": true})),
            Ok(SayRequest {
                speech: "salut".to_owned(),
                caption: Some("coucou".to_owned()),
                anonymous: true,
            })
        );
        // A blank caption is no caption; a long one is cut.
        let request = parse_request(&json!({"text": "salut", "caption": "   "})).unwrap();
        assert_eq!(request.caption, None);
        let long = "é".repeat(MAX_MEDIA_TEXT_CHARS + 10);
        let request = parse_request(&json!({"text": "salut", "caption": long})).unwrap();
        assert_eq!(
            request.caption.map(|caption| caption.chars().count()),
            Some(MAX_MEDIA_TEXT_CHARS)
        );
    }

    #[test]
    fn the_limit_counts_characters_after_collapsing() {
        let exact = "é".repeat(MAX_SPEECH_CHARS);
        assert!(parse_request(&json!({"text": format!("  {exact}  ")})).is_ok());
        let over = "é".repeat(MAX_SPEECH_CHARS + 1);
        assert_eq!(
            parse_request(&json!({"text": over})),
            Err(SayRejection::TooLong)
        );
    }

    #[test]
    fn invalid_bodies_are_refused() {
        for body in [
            json!([]),
            json!({}),
            json!({"text": 12}),
            json!({"text": " \n\t "}),
            json!({"text": "ok", "caption": 3}),
            json!({"text": "ok", "anonymous": "yes"}),
        ] {
            assert!(
                matches!(parse_request(&body), Err(SayRejection::Invalid(_))),
                "{body}"
            );
        }
    }
}
