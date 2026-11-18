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
use lc_db::{Db, MemberChange};
use lc_domain::rights::{Action, RightsTable, Surface};
use lc_domain::{
    GuildAccess, GuildId, GuildInvite, GuildMember, GuildMemberId, GuildRole, PairingCode,
    Permission,
};
use lc_protocol::{close_code, close_reason};
use rand::Rng as _;
use serde::Deserialize;
use serde_json::{json, Value};
use time::{Duration, UtcDateTime};

use crate::app::AppState;
use crate::discord::profiles::{refresh_member, RefreshOutcome};

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
pub(super) fn require(
    state: &AppState,
    auth: &AuthedOverlayClient,
    permission: Permission,
) -> Result<(), Response> {
    let action = Action::from_overlay_permission(permission);
    if state
        .rights
        .allowed(Surface::Overlay, action, auth.role().into())
    {
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
    pub expires_at: Option<UtcDateTime>,
}

/// Crée une invitation avec un code neuf.
///
/// Le code ne doit entrer en collision ni avec une autre invitation ni avec un
/// code d'appairage Discord (`pairing_codes`) : `POST /overlay/pair/consume`
/// cherche dans les deux tables.
pub(crate) async fn create_invite(
    db: &Db,
    invite: NewInvite,
    now: UtcDateTime,
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
pub(super) fn iso(date: Option<UtcDateTime>) -> Value {
    match date {
        Some(date) => json!(lc_domain::time::to_rfc3339_millis(date)),
        None => Value::Null,
    }
}

/// Payload JSON d'une invitation.
pub(crate) fn invite_payload(invite: &GuildInvite, now: UtcDateTime) -> Value {
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
///
/// `profileSource`: `discord` while the name and avatar follow the linked
/// Discord account, `custom` otherwise. `discordLinked` tells whether « Mon
/// profil » can switch back to Discord.
pub(super) fn member_payload(member: &GuildMember) -> Value {
    json!({
        "id": member.id.to_canonical(),
        "displayName": member.display_name,
        "avatar": member.avatar,
        "role": member.role.as_wire_str(),
        "createdAt": iso(Some(member.created_at)),
        "discordLinked": member.discord_user_id.is_some(),
        "discordUserId": member.discord_user_id,
        "profileSource": if member.follows_discord() { "discord" } else { "custom" },
    })
}

/// Droits effectifs du client, pour que l'interface masque ce qui est refusé.
fn permissions_payload(
    rights: &RightsTable,
    role: GuildRole,
    guests_can_view_board: bool,
) -> Value {
    let rights_role = role.into();
    let can = |action: Action| rights.allowed(Surface::Overlay, action, rights_role);
    json!({
        "viewBoard": can(Action::ViewBoard)
            && (role != GuildRole::Guest || guests_can_view_board),
        "viewQueue": can(Action::ViewQueue),
        "play": can(Action::TriggerPlayback),
        "stop": can(Action::StopPlayback),
        "addMedia": can(Action::AddMedia),
        "editAnyMedia": can(Action::EditAnyMedia),
        "moderateQueue": can(Action::ModerateQueue),
        "manageMembers": can(Action::ManageMembers),
        "manageInvites": can(Action::ManageInvites),
        "manageSettings": can(Action::ManageSettings),
    })
}

/// Builds the fixed-size Discord CDN URL for a guild icon.
///
/// Returns `None` unless `discord_guild_id` is a non-empty decimal snowflake
/// and `icon_hash` is 1 to 64 ASCII alphanumeric or underscore characters.
/// Animated hashes (`a_...`) intentionally resolve to a static `.png`.
#[must_use]
pub(crate) fn discord_icon_url(discord_guild_id: &str, icon_hash: &str) -> Option<String> {
    let valid_id =
        !discord_guild_id.is_empty() && discord_guild_id.bytes().all(|byte| byte.is_ascii_digit());
    let valid_hash = (1..=64).contains(&icon_hash.len())
        && icon_hash
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    (valid_id && valid_hash).then(|| {
        format!("https://cdn.discordapp.com/icons/{discord_guild_id}/{icon_hash}.png?size=128")
    })
}

/// Resolves the icon URL of a guild from its stored access row.
///
/// Native guilds (not standalone) use their own id as the Discord snowflake;
/// standalone guilds use their linked `discord_guild_id`.
#[must_use]
pub(crate) fn guild_icon_url(access: &GuildAccess) -> Option<String> {
    let icon_hash = access.discord_icon_hash.as_deref()?;
    let discord_guild_id = if access.standalone {
        access.discord_guild_id.as_deref()?
    } else {
        access.id.as_str()
    };
    discord_icon_url(discord_guild_id, icon_hash)
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
    let rights = state.rights.snapshot();

    json_response(
        StatusCode::OK,
        json!({
            "guild": {
                "id": guild_id.as_str(),
                "name": access.as_ref().and_then(|access| access.name.clone()),
                "standalone": access.as_ref().is_some_and(|access| access.standalone),
                "discordGuildId": access.as_ref().and_then(|access| access.discord_guild_id.clone()),
                "iconUrl": access.as_ref().and_then(guild_icon_url),
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
            "permissions": permissions_payload(&rights, role, guests_can_view_board),
        }),
    )
}

/// Corps `PATCH /overlay/profile`.
///
/// `useDiscord: true` makes a linked member follow their Discord account
/// again (the other fields are then ignored); any other change fixes the
/// profile the member chose.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileRequest {
    #[serde(default)]
    use_discord: bool,
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
    if payload.use_discord {
        return follow_discord(&state, db, member).await;
    }

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

    let saved = GuildMember {
        display_name,
        avatar,
        profile_custom: true,
        ..member.clone()
    };
    json_response(
        StatusCode::OK,
        json!({
            "updated": true,
            "member": member_payload(&saved),
        }),
    )
}

/// `PATCH /overlay/profile` with `useDiscord`: the member follows their
/// Discord account again and their profile is read from Discord at once.
/// `synced: false` when Discord could not be read: the next Discord command
/// or overlay connection will.
async fn follow_discord(state: &AppState, db: &Db, member: &GuildMember) -> Response {
    if member.discord_user_id.is_none() {
        return error(StatusCode::CONFLICT, "discord_not_linked");
    }
    match db.guild_members().follow_discord(member.id).await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::NOT_FOUND, "member_not_found"),
        Err(db_error) => return db_failure("overlay_profile.follow_discord", &db_error),
    }
    let following = GuildMember {
        profile_custom: false,
        discord_synced_at: None,
        ..member.clone()
    };
    let outcome = refresh_member(
        db,
        state.discord_profiles.as_ref(),
        &following,
        now(state),
        true,
    )
    .await;
    let stored = match db.guild_members().get(member.id).await {
        Ok(Some(stored)) => stored,
        Ok(None) => return error(StatusCode::NOT_FOUND, "member_not_found"),
        Err(db_error) => return db_failure("overlay_profile.reload", &db_error),
    };
    json_response(
        StatusCode::OK,
        json!({
            "updated": true,
            "synced": outcome == RefreshOutcome::Updated,
            "member": member_payload(&stored),
        }),
    )
}

/// Issues a device code binding an extra overlay to `member_id`.
///
/// Shared by `POST /overlay/device-code` (member of the calling client) and the
/// admin route (member supplied in the body). `created_by_member_id` records
/// who requested it; the admin console passes `None`.
pub(crate) async fn create_device_code_payload(
    state: &AppState,
    guild_id: &GuildId,
    member_id: GuildMemberId,
    role: GuildRole,
    created_by_member_id: Option<GuildMemberId>,
) -> Result<Value, Response> {
    let db = db(state)?;
    let now = now(state);
    let invite = NewInvite {
        guild_id: guild_id.clone(),
        role,
        member_id: Some(member_id),
        max_uses: Some(1),
        created_by_member_id,
        expires_at: Some(now + Duration::minutes(DEVICE_CODE_TTL_MINUTES)),
    };
    match create_invite(db, invite, now).await {
        Ok(Some(invite)) => Ok(json!({"created": true, "invite": invite_payload(&invite, now)})),
        Ok(None) => Err(internal_error()),
        Err(db_error) => Err(db_failure("overlay_device_code.create", &db_error)),
    }
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
    match create_device_code_payload(
        &state,
        auth.guild_id(),
        member.id,
        member.role,
        Some(member.id),
    )
    .await
    {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Authors under which a member's ingest clients and pending codes are
/// recorded: `member:<id>` for codes issued from the overlay, plus the linked
/// Discord account for codes issued with `/pair-code`.
pub(crate) fn member_code_authors(member: &GuildMember) -> Vec<String> {
    let mut authors = vec![format!(
        "{}{}",
        super::ingest_code::INGEST_CODE_CREATOR_PREFIX,
        member.id.to_canonical()
    )];
    if let Some(discord_user_id) = member.discord_user_id.as_deref() {
        authors.push(discord_user_id.to_owned());
    }
    authors
}

/// Retire un membre : marque le départ, révoque ses overlays et ferme leurs
/// sessions WS (`4402`), révoque ses extensions d'ingestion, ses codes en
/// attente et les invitations qu'il a émises.
///
/// Partagé par le départ volontaire, l'exclusion et le bannissement
/// ([`super::admin`]). The close reason is supplied by the caller.
pub(super) async fn remove_member(
    state: &AppState,
    db: &Db,
    member: &GuildMember,
    at: UtcDateTime,
    close_reason: &str,
) -> Result<MemberChange, Response> {
    let change = db
        .guild_members()
        .remove(member.id, at)
        .await
        .map_err(|db_error| db_failure("overlay_member.remove", &db_error))?;
    if change != MemberChange::Applied {
        return Ok(change);
    }
    let revoked = db
        .overlay_clients()
        .revoke_by_member(member.id, at)
        .await
        .map_err(|db_error| db_failure("overlay_member.revoke_clients", &db_error))?;
    for client_id in revoked {
        state.admin_hub.close_client_with_reason(
            &client_id.as_uuid(),
            close_code::REVOKED,
            close_reason,
        );
    }
    let authors = member_code_authors(member);
    db.ingest_clients()
        .revoke_by_authors(&member.guild_id, &authors, at)
        .await
        .map_err(|db_error| db_failure("overlay_member.revoke_ingest_clients", &db_error))?;
    db.pairing_codes()
        .revoke_by_authors(&member.guild_id, &authors, at)
        .await
        .map_err(|db_error| db_failure("overlay_member.revoke_pairing_codes", &db_error))?;
    db.guild_invites()
        .revoke_by_member(&member.guild_id, member.id, at)
        .await
        .map_err(|db_error| db_failure("overlay_member.revoke_invites", &db_error))?;
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
                state.admin_hub.close_client_with_reason(
                    &auth.client.id.as_uuid(),
                    close_code::REVOKED,
                    close_reason::REVOKED,
                );
                json_response(StatusCode::OK, json!({"left": true}))
            }
            Err(db_error) => db_failure("overlay_leave.revoke_client", &db_error),
        };
    };

    match remove_member(&state, db, member, now, close_reason::REVOKED).await {
        Ok(MemberChange::Applied) => json_response(StatusCode::OK, json!({"left": true})),
        Ok(MemberChange::LastAdmin) => error(StatusCode::CONFLICT, "last_admin"),
        Ok(MemberChange::NotFound) => error(StatusCode::NOT_FOUND, "member_not_found"),
        Err(response) => response,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{
        discord_icon_url, normalize_avatar, normalize_display_name, random_invite_code,
        CODE_ALPHABET,
    };

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

    #[test]
    fn discord_icon_url_accepts_valid_snowflakes_and_hashes() {
        assert_eq!(
            discord_icon_url("123456789012345678", "abcdef_0123456789").as_deref(),
            Some(
                "https://cdn.discordapp.com/icons/123456789012345678/abcdef_0123456789.png?size=128"
            )
        );
        assert_eq!(
            discord_icon_url("42", "a_0123456789abcdef").as_deref(),
            Some("https://cdn.discordapp.com/icons/42/a_0123456789abcdef.png?size=128")
        );
        let long_hash = "z".repeat(64);
        let expected = format!("https://cdn.discordapp.com/icons/42/{long_hash}.png?size=128");
        assert_eq!(
            discord_icon_url("42", &long_hash).as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn discord_icon_url_rejects_invalid_ids_and_hashes() {
        assert_eq!(discord_icon_url("", "abc"), None);
        assert_eq!(discord_icon_url("123abc", "abc"), None);
        assert_eq!(discord_icon_url("123", ""), None);
        assert_eq!(discord_icon_url("123", "ab-cd"), None);
        assert_eq!(discord_icon_url("123", "abc def"), None);
        assert_eq!(discord_icon_url("123", &"z".repeat(65)), None);
    }
}
