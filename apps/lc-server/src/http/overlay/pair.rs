//! `POST /overlay/pair/consume` — consommation d'un code d'appairage.
//!
//! Parité avec `server/src/components/overlay/overlayRoutes.ts:202-291`.
//!
//! ## Corps de requête
//!
//! ```json
//! { "code": "ABC123", "deviceName": "Poste salon", "appVersion": "2.0.1" }
//! ```
//!
//! - `code` : obligatoire, insensible à la casse et aux espaces de bord, puis
//!   normalisé en MAJUSCULES ;
//! - `deviceName` : facultatif ; repli legacy `Overlay-<authorName|User>` ;
//! - `label` / `clientName` sont acceptés en alias de `deviceName` (anciens
//!   clients) ;
//! - `displayName` / `avatar` : profil du nouveau membre venu d'une invitation
//!   (board, panel) ou d'un code `/pair-code` en mode invité ; sans pseudo la
//!   requête est refusée `400 display_name_required` **sans consommer le
//!   code**. Un code « appareil » (`guild_invites.member_id`) rattache un
//!   membre existant et n'exige aucun pseudo ;
//! - a normal-mode `/pair-code` is personal: the overlay joins the member
//!   linked to its author's Discord account (`guild_members.discord_user_id`),
//!   created on the first pairing with the Discord name and avatar. That
//!   profile then follows Discord until the member picks their own in « Mon
//!   profil ». `displayName` / `avatar` are ignored for such a code. A
//!   read-only invitation is handed to someone else: its author's identity is
//!   never used;
//! - `appVersion` / `sessionMode` sont ignorés pour la décision : le rôle vient
//!   TOUJOURS du code persisté, jamais du client (un client ne peut pas
//!   s'auto-attribuer un rôle).
//!
//! ## Deux familles de codes
//!
//! Le code est cherché d'abord dans `guild_invites` (invitations porteuses d'un
//! rôle, émises depuis l'overlay ou le panel serveur), puis dans
//! `pairing_codes` (codes émis depuis Discord : `NORMAL` → membre,
//! `INVITE_READ_ONLY` → invité, `grants_admin` → admin). Le code est **lu
//! avant toute décision** : un refus (pseudo manquant, code réservé à
//! l'extension) ne consomme ni l'invitation ni le code. Les tirets saisis
//! (`K7QX-M2PD`) sont ignorés.
//!
//! Depuis `/overlay/ingest-code`, un code `pairing_codes` peut être émis par un
//! membre overlay (`created_by_discord_user_id` préfixé `member:`) : il est
//! réservé à l'extension d'ingestion et l'overlay le refuse (`403
//! pairing_code_scope_mismatch`) **sans le consommer**.
//!
//! Chaque appairage rattache l'overlay à un membre de guilde : un nouveau
//! membre, ou le membre existant visé par un code « appareil »
//! (`guild_invites.member_id`).
//!
//! ## Réponse (200)
//!
//! ```json
//! {
//!   "clientToken": "…64 hex…",
//!   "clientId": "…uuid…",
//!   "guildId": "123456789012345678",
//!   "apiBaseUrl": "https://api.exemple.tld",
//!   "deviceName": "Poste salon",
//!   "authorName": "Alice" | null,
//!   "sessionMode": "normal" | "invite_read_only",
//!   "role": "admin" | "member" | "guest",
//!   "memberId": "…uuid…",
//!   "displayName": "Alice",
//!   "guildName": "Les Sardines" | null,
//!   "protocolVersion": 2
//! }
//! ```
//!
//! `protocolVersion` est un ajout additif v2 (le legacy ne le renvoyait pas) :
//! il permet au client Tauri de refuser un serveur incompatible avant le
//! handshake WebSocket. `clientToken` est la seule fois où le token brut existe
//! côté serveur ; il n'est jamais relu (seul son SHA-256 est stocké).
//!
//! ## Erreurs
//!
//! | Statut | Corps |
//! |--------|-------|
//! | 400 | `{"error":"invalid_payload"}` (code absent) |
//! | 400 | `{"error":"display_name_required"}` (nouveau membre sans pseudo utilisable) |
//! | 403 | `{"error":"pairing_code_scope_mismatch"}` (code réservé à l'extension) |
//! | 403 | `{"error":"member_banned"}` (code émis par un compte Discord banni de la guilde) |
//! | 404 | `{"error":"pairing_code_invalid_or_expired"}` |
//! | 409 | `{"error":"overlay_client_conflict"}` (collision de token, très improbable) |
//! | 500 | `{"error":"internal_error"}` |
//! | 503 | `{"error":"database_unavailable"}` |

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use lc_domain::{
    GuildId, GuildMember, GuildMemberId, GuildRole, OverlayClient, OverlayClientId, PairingCode,
    PairingCodeEntry, PairingMode,
};

use crate::app::AppState;

use super::guild::{normalize_avatar, normalize_display_name};
use super::{db, db_failure, error, is_conflict, json_response, now};

/// Discord identity carried by a personal `/pair-code`.
struct DiscordIdentity {
    guild_id: GuildId,
    user_id: String,
    name: String,
    image: Option<String>,
}

impl DiscordIdentity {
    /// Identity of a normal-mode code issued by a Discord account. `None`
    /// for read-only invitations (handed to someone else), browser-extension
    /// codes (`member:` author) and codes without a usable author name.
    fn from_code(entry: &PairingCodeEntry) -> Option<Self> {
        let user_id = entry.created_by_discord_user_id.as_str();
        let is_snowflake = !user_id.is_empty() && user_id.bytes().all(|byte| byte.is_ascii_digit());
        if entry.mode != PairingMode::Normal || !is_snowflake {
            return None;
        }
        Some(Self {
            guild_id: entry.guild_id.clone(),
            user_id: user_id.to_owned(),
            name: entry
                .author_name
                .as_deref()
                .and_then(normalize_display_name)?,
            image: entry.author_image.as_deref().and_then(normalize_avatar),
        })
    }
}

/// Ce qu'un code consommé accorde, quelle que soit sa famille.
struct Grant {
    guild_id: GuildId,
    role: GuildRole,
    /// Membre existant auquel rattacher l'overlay (code « appareil »).
    attach_member_id: Option<GuildMemberId>,
    /// Nom d'auteur porté par un code legacy.
    author_name: Option<String>,
    /// Auteur Discord d'un code legacy.
    created_by_discord_user_id: Option<String>,
}

/// Corps d'appairage (tolérant : les alias legacy sont acceptés).
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PairConsumeRequest {
    /// Code d'appairage saisi par l'utilisateur.
    #[serde(default)]
    code: Option<String>,
    /// Nom d'appareil demandé (nom de la fenêtre/du poste).
    #[serde(default, alias = "label", alias = "clientName")]
    device_name: Option<String>,
    /// Pseudo du nouveau membre.
    #[serde(default)]
    display_name: Option<String>,
    /// Avatar du nouveau membre (URL ou image embarquée).
    #[serde(default)]
    avatar: Option<String>,
    /// Version de l'application overlay (journalisation uniquement).
    #[serde(default)]
    app_version: Option<String>,
    /// Ignoré : le mode effectif est celui du code persisté.
    #[serde(default)]
    /// Champ legacy accepté puis ignoré : le mode vient du code persisté.
    #[allow(dead_code)]
    session_mode: Option<String>,
}

/// Longueur maximale d'un libellé de client (garde-fou base/affichage).
const MAX_LABEL_LEN: usize = 200;

/// Handler `POST /overlay/pair/consume`.
pub async fn consume_pairing(
    State(state): State<AppState>,
    payload: Result<Json<PairConsumeRequest>, JsonRejection>,
) -> Response {
    // AUD-A2-009 : JSON malformé → `400 invalid_payload` dédié (comme le legacy
    // qui traite un corps illisible comme une charge utile invalide), au lieu de
    // la réécriture générique `request_error`.
    let payload = match payload {
        Ok(Json(payload)) => payload,
        Err(rejection) => {
            tracing::debug!(
                error = %rejection.body_text(),
                "corps JSON invalide sur POST /overlay/pair/consume"
            );
            return error(StatusCode::BAD_REQUEST, "invalid_payload");
        }
    };
    let requested_code = payload
        .code
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    if requested_code.is_empty() {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    }

    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);

    // Nettoyage opportuniste des codes expirés (parité legacy) : un échec est
    // journalisé mais ne bloque pas la consommation du code courant.
    if let Err(db_error) = db.pairing_codes().delete_expired(now).await {
        tracing::warn!(error = %db_error, "nettoyage des codes d'appairage expirés impossible");
    }

    let code = match PairingCode::parse(&requested_code.replace('-', "")) {
        Ok(code) => code,
        Err(_) => return error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired"),
    };

    // Le code est d'abord LU, jamais consommé avant la décision : un refus
    // (`display_name_required` notamment) doit laisser l'invitation ou le code
    // intact.
    let invite = match db.guild_invites().get(&code).await {
        Ok(Some(invite)) if invite.is_usable_at(now) => Some(invite),
        Ok(_) => None,
        Err(db_error) => return db_failure("overlay_pair.lookup_invite", &db_error),
    };
    let pairing_entry = match invite {
        Some(_) => None,
        None => match db.pairing_codes().get(&code).await {
            Ok(Some(entry)) if entry.is_usable_at(now) => {
                // Un code émis depuis l'overlay (`member:`) est réservé à
                // l'extension : le refuser sans le consommer, sinon il
                // deviendrait inutilisable pour elle.
                if entry
                    .created_by_discord_user_id
                    .starts_with(super::ingest_code::INGEST_CODE_CREATOR_PREFIX)
                {
                    return error(StatusCode::FORBIDDEN, "pairing_code_scope_mismatch");
                }
                // A code issued by a banned Discord account (a read-only
                // invitation it made for itself, or one issued before the
                // ban) never lets that person back in.
                match db
                    .guild_bans()
                    .is_discord_user_banned(&entry.guild_id, &entry.created_by_discord_user_id)
                    .await
                {
                    Ok(true) => return error(StatusCode::FORBIDDEN, "member_banned"),
                    Ok(false) => {}
                    Err(db_error) => return db_failure("overlay_pair.ban_lookup", &db_error),
                }
                Some(entry)
            }
            Ok(_) => None,
            Err(db_error) => return db_failure("overlay_pair.lookup_code", &db_error),
        },
    };
    if invite.is_none() && pairing_entry.is_none() {
        return error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired");
    }

    let requested_display_name = payload
        .display_name
        .as_deref()
        .and_then(normalize_display_name);
    let requested_avatar = payload.avatar.as_deref().and_then(normalize_avatar);

    // A personal `/pair-code` brings its author's Discord identity, and the
    // member already linked to that account, if any.
    let discord_identity = pairing_entry.as_ref().and_then(DiscordIdentity::from_code);
    let linked_member_id = match &discord_identity {
        Some(identity) => match db
            .guild_members()
            .find_active_by_discord_user(&identity.guild_id, &identity.user_id)
            .await
        {
            Ok(member) => member.map(|member| member.id),
            Err(db_error) => return db_failure("overlay_pair.linked_member", &db_error),
        },
        None => None,
    };

    // Un code « appareil » vise un membre existant et n'a besoin d'aucun nom.
    let attach_member_id = invite.as_ref().and_then(|invite| invite.member_id);
    if attach_member_id.is_none() && discord_identity.is_none() && requested_display_name.is_none()
    {
        return error(StatusCode::BAD_REQUEST, "display_name_required");
    }

    let grant = match (invite, pairing_entry) {
        (Some(_), _) => match db.guild_invites().consume(&code, now).await {
            Ok(Some(invite)) => Grant {
                guild_id: invite.guild_id,
                role: invite.role,
                attach_member_id: invite.member_id,
                author_name: None,
                created_by_discord_user_id: None,
            },
            Ok(None) => return error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired"),
            Err(db_error) => return db_failure("overlay_pair.consume_invite", &db_error),
        },
        (None, Some(_)) => match db.pairing_codes().consume(&code, now).await {
            Ok(Some(entry)) => Grant {
                guild_id: entry.guild_id,
                role: if entry.grants_admin {
                    GuildRole::Admin
                } else {
                    GuildRole::from_pairing_mode(entry.mode)
                },
                // Pairing again with one's own `/pair-code` adds a device to
                // the same member (role and profile kept).
                attach_member_id: linked_member_id,
                author_name: entry.author_name,
                created_by_discord_user_id: Some(entry.created_by_discord_user_id),
            },
            Ok(None) => return error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired"),
            Err(db_error) => return db_failure("overlay_pair.consume_code", &db_error),
        },
        (None, None) => return error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired"),
    };

    let requested_label = payload
        .device_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(truncate_label);

    // Membre propriétaire de l'overlay : existant (code « appareil ») ou neuf.
    let (member, is_new_member) = match grant.attach_member_id {
        Some(member_id) => match db.guild_members().get(member_id).await {
            Ok(Some(member)) if member.is_active() => (member, false),
            Ok(_) => return error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired"),
            Err(db_error) => return db_failure("overlay_pair.member_lookup", &db_error),
        },
        None => {
            // Profile: the Discord identity of a personal `/pair-code`, else
            // the name typed in the overlay. Neither the device name, an
            // overlay label nor « User » ever names a member.
            let (display_name, avatar, discord_user_id) = match discord_identity {
                Some(identity) => (identity.name, identity.image, Some(identity.user_id)),
                None => match requested_display_name.clone() {
                    Some(display_name) => (display_name, requested_avatar.clone(), None),
                    None => return error(StatusCode::BAD_REQUEST, "display_name_required"),
                },
            };
            let linked = discord_user_id.is_some();
            let member = GuildMember {
                id: GuildMemberId::new_v4(),
                guild_id: grant.guild_id.clone(),
                display_name,
                avatar,
                role: grant.role,
                created_at: now,
                removed_at: None,
                discord_user_id,
                profile_custom: !linked,
                discord_synced_at: linked.then_some(now),
            };
            (member, true)
        }
    };
    // Un code « appareil » reprend le rôle courant du membre, pas celui figé
    // dans le code à sa création.
    let role = member.role;
    let session_mode = role.session_mode();

    // Parité legacy : le nom d'auteur renvoyé est celui du code Discord quand
    // il existe ; sinon c'est le pseudo du membre (le client, lui, reçoit
    // toujours le profil du membre dans `default_author_*`).
    let explicit_label = requested_label.is_some();
    let label = requested_label.unwrap_or_else(|| {
        let suffix = grant.author_name.as_deref().unwrap_or("User");
        truncate_label(&format!("Overlay-{suffix}"))
    });

    // Un poste ne garde qu'une session active par libellé. Pour un code legacy
    // la règle porte sur toute la guilde (parité) ; pour une invitation elle se
    // limite aux overlays du même membre, sinon un simple nom d'appareil
    // suffirait à déconnecter l'overlay de quelqu'un d'autre ; et elle ne joue
    // que pour un nom d'appareil fourni, sinon le libellé par défaut d'un
    // second overlay révoquerait le premier.
    let previous = if grant.created_by_discord_user_id.is_some() {
        db.overlay_clients().list_by_guild(&grant.guild_id).await
    } else if is_new_member || !explicit_label {
        Ok(Vec::new())
    } else {
        db.overlay_clients().list_active_by_member(member.id).await
    };
    match previous {
        Ok(clients) => {
            for client in clients {
                if client.is_active() && client.label == label {
                    if let Err(db_error) = db.overlay_clients().revoke(client.id, now).await {
                        return db_failure("overlay_pair.revoke_previous_client", &db_error);
                    }
                }
            }
        }
        Err(db_error) => return db_failure("overlay_pair.list_previous_clients", &db_error),
    }

    if is_new_member {
        if let Err(db_error) = db.guild_members().create(&member).await {
            // Two first pairings of one Discord account at the same time.
            if is_conflict(&db_error) {
                return error(StatusCode::CONFLICT, "overlay_client_conflict");
            }
            return db_failure("overlay_pair.create_member", &db_error);
        }
    }

    let token = super::auth::generate_token();
    let client = OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: grant.guild_id.clone(),
        label: label.clone(),
        token_hash: super::auth::hash_token(&token),
        session_mode,
        default_author_name: Some(member.display_name.clone()),
        default_author_image: member.avatar.clone(),
        created_by_discord_user_id: grant.created_by_discord_user_id.clone(),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    };
    if let Err(db_error) = db.overlay_clients().create(&client).await {
        if is_conflict(&db_error) {
            return error(StatusCode::CONFLICT, "overlay_client_conflict");
        }
        return db_failure("overlay_pair.create_client", &db_error);
    }
    if let Err(db_error) = db.overlay_clients().set_member(client.id, member.id).await {
        return db_failure("overlay_pair.attach_member", &db_error);
    }

    let guild_name = match db.guilds().access(&grant.guild_id).await {
        Ok(access) => access.and_then(|access| access.name),
        Err(db_error) => return db_failure("overlay_pair.guild_lookup", &db_error),
    };

    tracing::info!(
        client_id = %client.id,
        guild_id = %client.guild_id,
        member_id = %member.id,
        role = role.as_wire_str(),
        session_mode = %session_mode.as_wire_str(),
        app_version = payload.app_version.as_deref().unwrap_or(""),
        "overlay appairé"
    );

    json_response(
        StatusCode::OK,
        serde_json::json!({
            "clientToken": token,
            "clientId": client.id.to_canonical(),
            "guildId": client.guild_id.as_str(),
            "apiBaseUrl": state.config.api_origin(),
            "deviceName": client.label,
            "authorName": grant.author_name,
            "sessionMode": session_mode.as_wire_str(),
            "role": role.as_wire_str(),
            "memberId": member.id.to_canonical(),
            "displayName": member.display_name,
            "guildName": guild_name,
            "protocolVersion": lc_protocol::PROTOCOL_VERSION,
        }),
    )
}

/// Tronque un libellé en respectant les frontières de caractères UTF-8.
fn truncate_label(raw: &str) -> String {
    let mut value = raw.trim().to_owned();
    if value.chars().count() > MAX_LABEL_LEN {
        value = value.chars().take(MAX_LABEL_LEN).collect();
    }
    value
}
