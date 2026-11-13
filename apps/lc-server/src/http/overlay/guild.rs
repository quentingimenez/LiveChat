//! Guilde vue par un overlay : identité, profil, départ, invitations.
//!
//! | Méthode | Chemin | Droit |
//! |---------|--------|-------|
//! | `GET` | `/overlay/guild` | tout membre |
//! | `PATCH` | `/overlay/profile` | tout membre (son propre profil) |
//! | `POST` | `/overlay/device-code` | tout membre (code pour un second overlay) |
//! | `POST` | `/overlay/leave` | tout membre |
//!
//! Ce module porte aussi les helpers partagés avec [`super::admin`] et
//! [`super::pair`] : garde de permission, normalisation du profil, génération
//! des codes d'invitation.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use lc_db::{Db, MemberChange};
use lc_domain::{
    GuildId, GuildInvite, GuildMember, GuildMemberId, GuildRole, PairingCode, Permission,
};
use lc_protocol::close_code;
use rand::Rng as _;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::AppState;

use super::auth::AuthedOverlayClient;
use super::{db, db_failure, error, internal_error, is_conflict, json_response, now};

/// Alphabet sans ambiguïté des codes (identique aux codes émis depuis Discord).
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Longueur d'un code d'invitation (40 bits d'entropie).
const INVITE_CODE_LEN: usize = 8;
/// Tentatives de génération avant d'abandonner (collision de clé primaire).
const INVITE_CODE_ATTEMPTS: usize = 5;
/// Longueur maximale d'un pseudo.
pub(super) const DISPLAY_NAME_MAX_CHARS: usize = 40;
/// Longueur maximale d'une URL d'avatar.
const AVATAR_URL_MAX_LEN: usize = 2_048;
/// Longueur maximale d'un avatar embarqué (`data:image/…;base64,`).
const AVATAR_DATA_MAX_LEN: usize = 96_000;
/// Durée de vie d'un code « appareil ».
const DEVICE_CODE_TTL_MINUTES: i64 = 10;

/// Garde de permission : `403 {"error":"forbidden"}` si le rôle ne suffit pas.
pub(super) fn require(auth: &AuthedOverlayClient, permission: Permission) -> Result<(), Response> {
    if auth.can(permission) {
        Ok(())
    } else {
        Err(error(StatusCode::FORBIDDEN, "forbidden"))
    }
}

/// Normalise un pseudo : espaces compactés, longueur bornée ; `None` si vide.
pub(crate) fn normalize_display_name(raw: &str) -> Option<String> {
    normalize_label(raw, DISPLAY_NAME_MAX_CHARS)
}

/// Normalise un libellé court (pseudo, nom de guilde) : espaces compactés,
/// `max_chars` caractères au plus ; `None` si vide.
pub(crate) fn normalize_label(raw: &str, max_chars: usize) -> Option<String> {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let value: String = compact.chars().take(max_chars).collect();
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

/// Valide un avatar : URL `http(s)` ou image embarquée de taille bornée.
///
/// Toute autre valeur est ignorée (`None`) plutôt que refusée : l'avatar est
/// décoratif et ne doit jamais faire échouer un appairage.
pub(super) fn normalize_avatar(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    let lower = value.get(..value.len().min(32))?.to_ascii_lowercase();
    let is_url = (lower.starts_with("https://") || lower.starts_with("http://"))
        && value.len() <= AVATAR_URL_MAX_LEN;
    let is_data = ["png", "jpeg", "webp", "gif"]
        .iter()
        .any(|kind| lower.starts_with(&format!("data:image/{kind};base64,")))
        && value.len() <= AVATAR_DATA_MAX_LEN;
    (is_url || is_data).then(|| value.to_owned())
}

/// Tire un code d'invitation aléatoire.
pub(super) fn random_invite_code() -> String {
    let mut rng = rand::rng();
    (0..INVITE_CODE_LEN)
        .map(|_| char::from(CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())]))
        .collect()
}

/// Paramètres d'une invitation à créer.
pub(crate) struct NewInvite {
    pub guild_id: GuildId,
    pub role: GuildRole,
    pub member_id: Option<GuildMemberId>,
    pub max_uses: Option<i64>,
    pub created_by_member_id: Option<GuildMemberId>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Crée une invitation avec un code neuf.
///
/// Le code ne doit entrer en collision ni avec une autre invitation ni avec un
/// code d'appairage Discord (`pairing_codes`) : `POST /overlay/pair/consume`
/// cherche dans les deux tables.
pub(crate) async fn create_invite(
    db: &Db,
    invite: NewInvite,
    now: DateTime<Utc>,
) -> lc_db::DbResult<Option<GuildInvite>> {
    for _ in 0..INVITE_CODE_ATTEMPTS {
        let Ok(code) = PairingCode::parse(&random_invite_code()) else {
            continue;
        };
        if db.pairing_codes().get(&code).await?.is_some() {
            continue;
        }
        let entity = GuildInvite {
            code,
            guild_id: invite.guild_id.clone(),
            role: invite.role,
            member_id: invite.member_id,
            max_uses: invite.max_uses,
            use_count: 0,
            created_by_member_id: invite.created_by_member_id,
            expires_at: invite.expires_at,
            revoked_at: None,
            created_at: now,
        };
        match db.guild_invites().create(&entity).await {
            Ok(()) => return Ok(Some(entity)),
            Err(db_error) if is_conflict(&db_error) => {}
            Err(db_error) => return Err(db_error),
        }
    }
    Ok(None)
}

/// Date ISO 8601 (millisecondes, UTC) ou `null`.
pub(super) fn iso(date: Option<DateTime<Utc>>) -> Value {
    match date {
        Some(date) => json!(date.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        None => Value::Null,
    }
}

/// Payload JSON d'une invitation.
pub(crate) fn invite_payload(invite: &GuildInvite, now: DateTime<Utc>) -> Value {
    json!({
        "code": invite.code.as_str(),
        "role": invite.role.as_wire_str(),
        "memberId": invite.member_id.map(GuildMemberId::to_canonical),
        "maxUses": invite.max_uses,
        "useCount": invite.use_count,
        "createdByMemberId": invite.created_by_member_id.map(GuildMemberId::to_canonical),
        "expiresAt": iso(invite.expires_at),
        "revokedAt": iso(invite.revoked_at),
        "createdAt": iso(Some(invite.created_at)),
        "isActive": invite.is_usable_at(now),
    })
}

/// Payload JSON d'un membre (profil et rôle).
pub(super) fn member_payload(member: &GuildMember) -> Value {
    json!({
        "id": member.id.to_canonical(),
        "displayName": member.display_name,
        "avatar": member.avatar,
        "role": member.role.as_wire_str(),
        "createdAt": iso(Some(member.created_at)),
    })
}

/// Droits effectifs du client, pour que l'interface masque ce qui est refusé.
fn permissions_payload(role: GuildRole, guests_can_view_board: bool) -> Value {
    json!({
        "viewBoard": role.can_view_board(guests_can_view_board),
        "viewQueue": role.can(Permission::ViewQueue),
        "play": role.can(Permission::TriggerPlayback),
        "stop": role.can(Permission::StopPlayback),
        "addMedia": role.can(Permission::AddMedia),
        "editAnyMedia": role.can(Permission::EditAnyMedia),
        // Legacy overlay 3.0.0 compatibility: remove in 3.1.
        "addMeme": role.can(Permission::AddMedia),
        // Legacy overlay 3.0.0 compatibility: remove in 3.1.
        "editAnyMeme": role.can(Permission::EditAnyMedia),
        "moderateQueue": role.can(Permission::ModerateQueue),
        "manageMembers": role.can(Permission::ManageMembers),
        "manageInvites": role.can(Permission::ManageInvites),
        "manageSettings": role.can(Permission::ManageSettings),
    })
}

/// Handler `GET /overlay/guild`.
pub async fn get_guild(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let guild_id = auth.guild_id();
    let access = match db.guilds().access(guild_id).await {
        Ok(access) => access,
        Err(db_error) => return db_failure("overlay_guild.access", &db_error),
    };
    let settings = match db.guilds().get(guild_id).await {
        Ok(settings) => settings,
        Err(db_error) => return db_failure("overlay_guild.settings", &db_error),
    };
    let guests_can_view_board = access
        .as_ref()
        .is_none_or(|access| access.guests_can_view_board);
    let role = auth.role();

    json_response(
        StatusCode::OK,
        json!({
            "guild": {
                "id": guild_id.as_str(),
                "name": access.as_ref().and_then(|access| access.name.clone()),
                "standalone": access.as_ref().is_some_and(|access| access.standalone),
                "discordGuildId": access.as_ref().and_then(|access| access.discord_guild_id.clone()),
                "guestsCanViewBoard": guests_can_view_board,
                "settings": {
                    "defaultMediaTime": settings.as_ref().and_then(|guild| guild.default_media_time),
                    "maxMediaTime": settings.as_ref().and_then(|guild| guild.max_media_time),
                    "displayMediaFull": settings.as_ref().is_some_and(|guild| guild.display_media_full),
                },
                "connectedCount": state.admin_hub.connected_for_guild(guild_id),
            },
            "member": match &auth.member {
                Some(member) => member_payload(member),
                None => json!({
                    "id": Value::Null,
                    "displayName": auth.client.default_author_name.clone().unwrap_or_else(|| auth.client.label.clone()),
                    "avatar": auth.client.default_author_image,
                    "role": role.as_wire_str(),
                    "createdAt": iso(Some(auth.client.created_at)),
                }),
            },
            "clientId": auth.client.id.to_canonical(),
            "permissions": permissions_payload(role, guests_can_view_board),
        }),
    )
}

/// Corps `PATCH /overlay/profile`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileRequest {
    #[serde(default)]
    display_name: Option<String>,
    /// `null` ou chaîne vide efface l'avatar ; absent le conserve.
    #[serde(default, deserialize_with = "double_option")]
    avatar: Option<Option<String>>,
}

/// Distingue un champ absent (`None`) d'un champ `null` (`Some(None)`).
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// Handler `PATCH /overlay/profile`.
pub async fn update_profile(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<ProfileRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let Some(member) = auth.member.as_ref() else {
        return error(StatusCode::CONFLICT, "member_required");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let display_name = match payload.display_name.as_deref() {
        Some(raw) => match normalize_display_name(raw) {
            Some(name) => name,
            None => return error(StatusCode::BAD_REQUEST, "invalid_display_name"),
        },
        None => member.display_name.clone(),
    };
    let avatar = match payload.avatar {
        Some(requested) => requested.as_deref().and_then(normalize_avatar),
        None => member.avatar.clone(),
    };

    match db
        .guild_members()
        .update_profile(member.id, &display_name, avatar.as_deref())
        .await
    {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::NOT_FOUND, "member_not_found"),
        Err(db_error) => return db_failure("overlay_profile.update", &db_error),
    }
    // Le nom d'auteur affiché à la lecture vient de l'overlay : on l'aligne.
    if let Err(db_error) = db
        .overlay_clients()
        .update_author_by_member(member.id, &display_name, avatar.as_deref())
        .await
    {
        return db_failure("overlay_profile.sync_clients", &db_error);
    }

    json_response(
        StatusCode::OK,
        json!({
            "updated": true,
            "member": {
                "id": member.id.to_canonical(),
                "displayName": display_name,
                "avatar": avatar,
                "role": member.role.as_wire_str(),
            },
        }),
    )
}

/// Handler `POST /overlay/device-code` : code à usage unique rattachant un
/// overlay supplémentaire au membre courant.
pub async fn create_device_code(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
) -> Response {
    let Some(member) = auth.member.as_ref() else {
        return error(StatusCode::CONFLICT, "member_required");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);
    let invite = NewInvite {
        guild_id: member.guild_id.clone(),
        role: member.role,
        member_id: Some(member.id),
        max_uses: Some(1),
        created_by_member_id: Some(member.id),
        expires_at: Some(now + Duration::minutes(DEVICE_CODE_TTL_MINUTES)),
    };
    match create_invite(db, invite, now).await {
        Ok(Some(invite)) => json_response(
            StatusCode::OK,
            json!({"created": true, "invite": invite_payload(&invite, now)}),
        ),
        Ok(None) => internal_error(),
        Err(db_error) => db_failure("overlay_device_code.create", &db_error),
    }
}

/// Retire un membre : marque le départ, révoque ses overlays, ferme leurs
/// sessions WS (`4402 revoked`).
///
/// Partagé par le départ volontaire et l'exclusion ([`super::admin`]).
pub(super) async fn remove_member(
    state: &AppState,
    db: &Db,
    member_id: GuildMemberId,
    at: DateTime<Utc>,
) -> Result<MemberChange, Response> {
    let change = db
        .guild_members()
        .remove(member_id, at)
        .await
        .map_err(|db_error| db_failure("overlay_member.remove", &db_error))?;
    if change != MemberChange::Applied {
        return Ok(change);
    }
    let revoked = db
        .overlay_clients()
        .revoke_by_member(member_id, at)
        .await
        .map_err(|db_error| db_failure("overlay_member.revoke_clients", &db_error))?;
    for client_id in revoked {
        state
            .admin_hub
            .close_client(&client_id.as_uuid(), close_code::REVOKED);
    }
    Ok(change)
}

/// Handler `POST /overlay/leave`.
pub async fn leave(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);

    let Some(member) = auth.member.as_ref() else {
        // Overlay sans membre : seul cet overlay est révoqué.
        return match db.overlay_clients().revoke(auth.client.id, now).await {
            Ok(_) => {
                state
                    .admin_hub
                    .close_client(&auth.client.id.as_uuid(), close_code::REVOKED);
                json_response(StatusCode::OK, json!({"left": true}))
            }
            Err(db_error) => db_failure("overlay_leave.revoke_client", &db_error),
        };
    };

    match remove_member(&state, db, member.id, now).await {
        Ok(MemberChange::Applied) => json_response(StatusCode::OK, json!({"left": true})),
        Ok(MemberChange::LastAdmin) => error(StatusCode::CONFLICT, "last_admin"),
        Ok(MemberChange::NotFound) => error(StatusCode::NOT_FOUND, "member_not_found"),
        Err(response) => response,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{normalize_avatar, normalize_display_name, random_invite_code, CODE_ALPHABET};

    #[test]
    fn pseudo_normalise() {
        assert_eq!(
            normalize_display_name("  Léa   B.  ").as_deref(),
            Some("Léa B.")
        );
        assert_eq!(normalize_display_name("   "), None);
        // Un nom de guilde a sa propre limite, plus longue que celle d'un pseudo.
        assert_eq!(
            super::normalize_label(&"y".repeat(100), 60)
                .unwrap()
                .chars()
                .count(),
            60
        );
        assert_eq!(
            normalize_display_name(&"x".repeat(100))
                .unwrap()
                .chars()
                .count(),
            40
        );
    }

    #[test]
    fn avatar_valide_ou_ignore() {
        assert_eq!(
            normalize_avatar(" https://example.test/a.png ").as_deref(),
            Some("https://example.test/a.png")
        );
        assert!(normalize_avatar("data:image/png;base64,AAAA").is_some());
        assert_eq!(normalize_avatar("javascript:alert(1)"), None);
        assert_eq!(normalize_avatar("data:text/html;base64,AAAA"), None);
        assert_eq!(normalize_avatar(""), None);
        let huge = format!("data:image/png;base64,{}", "A".repeat(200_000));
        assert_eq!(normalize_avatar(&huge), None);
    }

    #[test]
    fn code_d_invitation_bien_forme() {
        let code = random_invite_code();
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|byte| CODE_ALPHABET.contains(&byte)));
        assert!(lc_domain::PairingCode::parse(&code).is_ok());
    }
}
