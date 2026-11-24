//! Entités du domaine, alignées sur le contrat `docs/rust/schema-v3.sql`.
//!
//! - noms de champs : `snake_case` comme en base (pas de renommage de colonne
//!   caché dans `lc-db`) ;
//! - dates : [`time::UtcDateTime`] en mémoire, sérialisées en millisecondes
//!   epoch UTF lorsque l'entité est exposée en JSON (`camelCase`) ;
//! - booléens : `bool` Rust, stockés `INTEGER 0/1` avec `CHECK` côté base.
//!
//! Aucune entité ne contient de logique de persistance : les repositories
//! `lc-db` convertissent `row <-> entité`.

use serde::{Deserialize, Serialize};
use time::UtcDateTime;

use crate::enums::{
    GuildRole, JobSource, JobStatus, MediaKind, MediaStatus, OverlaySessionMode, PairingMode,
};
use crate::ids::{
    GuildBanId, GuildId, GuildMemberId, IngestClientId, MediaAssetId, MediaBoardItemId,
    OverlayClientId, PairingCode as PairingCodeValue, PlaybackJobId,
};

/// Module serde pour les dates obligatoires en ms epoch UTC.
mod ms_required {
    use serde::{Deserialize, Deserializer, Serializer};
    use time::UtcDateTime;

    pub fn serialize<S>(value: &UtcDateTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serializer::serialize_i64(serializer, crate::time::datetime_to_ms(*value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<UtcDateTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = i64::deserialize(deserializer)?;
        crate::time::ms_to_datetime(millis)
            .ok_or_else(|| serde::de::Error::custom(format!("horodatage hors plage : {millis} ms")))
    }
}

/// Module serde pour les dates optionnelles en ms epoch UTC (`null` = absent).
mod ms_optional {
    use serde::{Deserialize, Deserializer, Serializer};
    use time::UtcDateTime;

    pub fn serialize<S>(value: &Option<UtcDateTime>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(date) => {
                Serializer::serialize_some(serializer, &crate::time::datetime_to_ms(*date))
            }
            None => Serializer::serialize_none(serializer),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<UtcDateTime>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = Option::<i64>::deserialize(deserializer)?;
        match millis {
            Some(millis) => crate::time::ms_to_datetime(millis)
                .map(Some)
                .ok_or_else(|| {
                    serde::de::Error::custom(format!("horodatage hors plage : {millis} ms"))
                }),
            None => Ok(None),
        }
    }
}

/// Média ingéré (vidéo/image/audio) et son cache local.
///
/// Le pipeline média (Phase 3, `lc-media`) crée la ligne en `PROCESSING`, puis
/// la passe en `READY` ou `FAILED` via `MediaAssetRepository::update_status`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAsset {
    /// Clé primaire UUID.
    pub id: MediaAssetId,
    /// Empreinte du contenu source (clé naturelle unique, dédoublonnage).
    pub source_hash: String,
    /// URL d'origine (plateforme, CDN…).
    pub source_url: String,
    /// Titre réel de la source (titre de la vidéo, texte du tweet, nom de
    /// fichier…), si le pipeline l'a trouvé. Sert de titre par défaut à un
    /// élément de board ajouté depuis un lien sans titre saisi.
    pub source_title: Option<String>,
    /// Famille du média.
    pub kind: MediaKind,
    /// Type MIME servi (`video/mp4`, `image/gif`…).
    pub mime: String,
    /// Durée en secondes (audio/vidéo ; `None` pour une image).
    pub duration_sec: Option<i64>,
    /// Largeur en pixels (si connue).
    pub width: Option<i64>,
    /// Hauteur en pixels (si connue).
    pub height: Option<i64>,
    /// Orientation verticale (portrait), utilisée par l'overlay.
    pub is_vertical: bool,
    /// Chemin local du fichier dans le cache média.
    pub storage_path: String,
    /// Taille du fichier en octets.
    pub size_bytes: i64,
    /// État du pipeline.
    pub status: MediaStatus,
    /// Détail d'échec quand `status == Failed`.
    pub error: Option<String>,
    /// Dernier accès (TTL glissant du cache).
    #[serde(with = "ms_required")]
    pub last_accessed_at: UtcDateTime,
    /// Date d'expiration du cache.
    #[serde(with = "ms_required")]
    pub expires_at: UtcDateTime,
    /// Date de création de la ligne.
    #[serde(with = "ms_required")]
    pub created_at: UtcDateTime,
}

impl MediaAsset {
    /// `true` si le média est prêt à être servi.
    pub fn is_ready(&self) -> bool {
        self.status == MediaStatus::Ready
    }

    /// `true` si le média est expiré à l'instant `now`.
    pub fn is_expired_at(&self, now: UtcDateTime) -> bool {
        self.expires_at <= now
    }
}

/// Élément du board média d'une guilde.
///
/// Contraintes du schéma : `UNIQUE (guild_id, media_asset_id)` et
/// `ON DELETE CASCADE` depuis `media_assets` (purger un média supprime les
/// entrées de board qui le référencent).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaBoardItem {
    /// Clé primaire UUID.
    pub id: MediaBoardItemId,
    /// Guilde Discord propriétaire.
    pub guild_id: GuildId,
    /// Média associé (aucun doublon par guilde).
    pub media_asset_id: MediaAssetId,
    /// Titre affiché sur le board.
    pub title: Option<String>,
    /// Texte/message associé au média.
    pub message: Option<String>,
    /// Auteur Discord (snowflake), si l'ajout vient de Discord.
    pub created_by_discord_user_id: Option<String>,
    /// Nom d'affichage de l'auteur, figé à la création.
    pub created_by_name: Option<String>,
    /// Date d'ajout au board.
    #[serde(with = "ms_required")]
    pub created_at: UtcDateTime,
    /// Picture of an audio item: its embedded cover by default, or an image
    /// chosen by a member.
    #[serde(default)]
    pub artwork_asset_id: Option<MediaAssetId>,
    /// `true` when overlays also show the picture while the audio plays.
    #[serde(default)]
    pub artwork_on_overlay: bool,
    /// Display time chosen for an image item, in seconds
    /// ([`MIN_DISPLAY_DURATION_SEC`]..=[`MAX_DISPLAY_DURATION_SEC`]); `None`
    /// uses the guild default. Ignored for video and audio.
    #[serde(default)]
    pub display_duration_sec: Option<i64>,
    /// Scale chosen for an image item, in percent
    /// ([`MIN_DISPLAY_SCALE_PERCENT`]..=[`MAX_DISPLAY_SCALE_PERCENT`]) of its
    /// natural size on the overlay. `None` is 100 %, today's behaviour.
    #[serde(default)]
    pub display_scale_percent: Option<i64>,
}

/// Shortest display time a member can choose for an image, in seconds.
pub const MIN_DISPLAY_DURATION_SEC: i64 = 1;
/// Longest display time a member can choose for an image, in seconds.
pub const MAX_DISPLAY_DURATION_SEC: i64 = 600;

/// `true` when `seconds` is a display time a member may choose.
pub fn is_valid_display_duration(seconds: i64) -> bool {
    (MIN_DISPLAY_DURATION_SEC..=MAX_DISPLAY_DURATION_SEC).contains(&seconds)
}

/// Smallest scale an image can be shown at, in percent of its natural size.
pub const MIN_DISPLAY_SCALE_PERCENT: i64 = 10;
/// Largest scale an image can be shown at, in percent of its natural size.
pub const MAX_DISPLAY_SCALE_PERCENT: i64 = 1000;

/// `true` when `percent` is a scale a member may choose.
pub fn is_valid_display_scale(percent: i64) -> bool {
    (MIN_DISPLAY_SCALE_PERCENT..=MAX_DISPLAY_SCALE_PERCENT).contains(&percent)
}

/// Job de la file de lecture (scheduler).
///
/// Invariant du schéma et du scheduler : **au plus un job `Playing` par
/// guilde** (garanti par `PlaybackJobRepository::claim_next`, cf.
/// `docs/rust/protocol-v2.md` §15.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackJob {
    /// Clé primaire UUID.
    pub id: PlaybackJobId,
    /// Guilde Discord cible.
    pub guild_id: GuildId,
    /// Média à lire (`None` pour un job texte seul).
    pub media_asset_id: Option<MediaAssetId>,
    /// Texte affiché pendant la lecture (média textuel).
    pub text: Option<String>,
    /// Afficher le texte en plus du média.
    pub show_text: bool,
    /// Nom d'auteur affiché par l'overlay.
    pub author_name: Option<String>,
    /// URL de l'avatar d'auteur affiché.
    pub author_image: Option<String>,
    /// Durée de lecture demandée (secondes).
    pub duration_sec: i64,
    /// Origine du job (board, ingestion, overlay, Discord).
    pub source: JobSource,
    /// État courant.
    pub status: JobStatus,
    /// Offset de départ en secondes (départ d'une URL YouTube, `!play`…).
    pub resume_offset_sec: i64,
    /// Dernier `playback.state` reçu (`playing`/`paused`).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub last_playback_state_at: Option<UtcDateTime>,
    /// Date de soumission (ordre FIFO, avec `id`).
    #[serde(with = "ms_required")]
    pub submission_date: UtcDateTime,
    /// Date d'exécution planifiée par le scheduler (legacy `executionDate`).
    #[serde(with = "ms_required")]
    pub execution_date: UtcDateTime,
    /// Date à partir de laquelle le job est éligible au claim.
    #[serde(with = "ms_required")]
    pub scheduled_at: UtcDateTime,
    /// Date de passage effectif en `Playing`.
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<UtcDateTime>,
    /// Date de passage en état terminal.
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<UtcDateTime>,
}

impl PlaybackJob {
    /// `true` si l'état est terminal (`Done`/`Failed`).
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// `true` si le job est éligible au claim à l'instant `now`
    /// (état `Pending` et `scheduled_at` atteint).
    pub fn is_claimable_at(&self, now: UtcDateTime) -> bool {
        self.status == JobStatus::Pending && self.scheduled_at <= now
    }
}

/// Réglages persistants d'une guilde.
///
/// La ligne est créée à la demande par `GuildRepository::upsert` (legacy :
/// `prisma.guild.upsert` à chaque commande Discord).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Guild {
    /// Snowflake Discord (clé primaire).
    pub id: GuildId,
    /// Nom lisible de la guilde (cache d'affichage).
    pub name: Option<String>,
    /// Verrou anti-collision : un nouveau job ne peut démarrer avant.
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub busy_until: Option<UtcDateTime>,
    /// Durée d'affichage par défaut d'un média (secondes), si surchargée.
    pub default_media_time: Option<i64>,
    /// Durée maximale autorisée d'un média (secondes), si surchargée.
    pub max_media_time: Option<i64>,
    /// Afficher les médias en plein écran.
    pub display_media_full: bool,
}

/// Rattachement d'une guilde : indépendante ou liée à un serveur Discord.
///
/// Colonnes ajoutées par la migration 0003, lues à part de [`Guild`] (qui reste
/// l'entité des réglages de lecture).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuildAccess {
    /// Identifiant de la guilde.
    pub id: GuildId,
    /// Nom lisible.
    pub name: Option<String>,
    /// `true` si la guilde a été créée hors Discord (panel serveur). Sinon son
    /// identifiant **est** le snowflake du serveur Discord (legacy).
    pub standalone: bool,
    /// Serveur Discord lié à une guilde indépendante, si elle en a un.
    pub discord_guild_id: Option<String>,
    /// Discord icon hash, when the linked server has an icon.
    pub discord_icon_hash: Option<String>,
    /// Les invités peuvent consulter le board média.
    pub guests_can_view_board: bool,
    /// Date de création (`None` pour une guilde antérieure à la migration 0003).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<UtcDateTime>,
}

/// Membre d'une guilde : un profil (pseudo, avatar) et un rôle.
///
/// Un membre possède un ou plusieurs overlays (`overlay_clients.member_id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuildMember {
    /// Clé primaire UUID.
    pub id: GuildMemberId,
    /// Guilde d'appartenance.
    pub guild_id: GuildId,
    /// Pseudo affiché.
    pub display_name: String,
    /// Avatar (URL), si renseigné.
    pub avatar: Option<String>,
    /// Rôle dans la guilde.
    pub role: GuildRole,
    /// Date d'arrivée.
    #[serde(with = "ms_required")]
    pub created_at: UtcDateTime,
    /// Date d'exclusion ou de départ (`None` = actif).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub removed_at: Option<UtcDateTime>,
    /// Discord account linked to the member (snowflake): set when the member
    /// paired with their own `/pair-code`. At most one active member per
    /// account and guild.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_user_id: Option<String>,
    /// `true` once the member chose their own name or avatar: Discord no
    /// longer overwrites the profile. Always `true` for an unlinked member.
    #[serde(default)]
    pub profile_custom: bool,
    /// Last time the profile was read from Discord.
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub discord_synced_at: Option<UtcDateTime>,
}

impl GuildMember {
    /// `true` si le membre n'a ni quitté la guilde ni été exclu.
    pub fn is_active(&self) -> bool {
        self.removed_at.is_none()
    }

    /// `true` while the name and avatar follow the linked Discord account.
    pub fn follows_discord(&self) -> bool {
        self.discord_user_id.is_some() && !self.profile_custom
    }
}

/// Invitation à rejoindre une guilde : le code porte le rôle accordé.
///
/// Si `member_id` est renseigné, le code n'ajoute pas un nouveau membre : il
/// rattache un overlay supplémentaire à ce membre existant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuildInvite {
    /// Code lisible (clé primaire, MAJUSCULES).
    pub code: PairingCodeValue,
    /// Guilde cible.
    pub guild_id: GuildId,
    /// Rôle accordé au nouveau membre.
    pub role: GuildRole,
    /// Membre existant auquel rattacher l'overlay (code « appareil »).
    pub member_id: Option<GuildMemberId>,
    /// Nombre maximal d'utilisations (`None` = illimité).
    pub max_uses: Option<i64>,
    /// Nombre d'utilisations déjà faites.
    pub use_count: i64,
    /// Membre créateur (`None` = émis par l'admin serveur).
    pub created_by_member_id: Option<GuildMemberId>,
    /// Expiration (`None` = jamais).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<UtcDateTime>,
    /// Date de révocation (`None` = non révoquée).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<UtcDateTime>,
    /// Date de création.
    #[serde(with = "ms_required")]
    pub created_at: UtcDateTime,
}

impl GuildInvite {
    /// `true` si l'invitation peut encore être utilisée à l'instant `now`.
    pub fn is_usable_at(&self, now: UtcDateTime) -> bool {
        self.revoked_at.is_none()
            && self.expires_at.is_none_or(|expires_at| expires_at > now)
            && self
                .max_uses
                .is_none_or(|max_uses| self.use_count < max_uses)
    }
}

/// Ban of a person from a guild.
///
/// Banning removes the member and revokes their overlays, ingest clients and
/// pending codes. The Discord account, when the member had linked one, is the
/// lasting identity: it can no longer obtain or redeem a code for this guild,
/// nor run the bot commands. A member without a linked account is only
/// removed: nothing identifies them on a later invitation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuildBan {
    /// Primary key (UUID).
    pub id: GuildBanId,
    /// Guild the person is banned from.
    pub guild_id: GuildId,
    /// Member that was banned (no foreign key: kept for history).
    pub member_id: Option<GuildMemberId>,
    /// Banned Discord account (snowflake), when the member had linked one.
    pub discord_user_id: Option<String>,
    /// Name of the member at ban time.
    pub display_name: String,
    /// Avatar of the member at ban time.
    pub avatar: Option<String>,
    /// Name of who banned (`None` = the server operator console).
    pub banned_by: Option<String>,
    /// Ban date.
    #[serde(with = "ms_required")]
    pub created_at: UtcDateTime,
}

/// Code de liaison Discord émis par l'overlay d'une guilde indépendante.
///
/// Un administrateur du serveur Discord saisit ce code avec la commande
/// `/lier` (`/link`) : `DiscordLinkCodesRepository::redeem` renseigne alors
/// `guilds.discord_guild_id`. Usage unique et durée de vie courte ; un nouveau
/// code émis remplace les codes non utilisés de la même guilde.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordLinkCode {
    /// Code lisible (clé primaire, MAJUSCULES).
    pub code: PairingCodeValue,
    /// Guilde indépendante à lier.
    pub guild_id: GuildId,
    /// Membre administrateur qui a émis le code (`None` = panel serveur).
    pub created_by_member_id: Option<GuildMemberId>,
    /// Expiration (obligatoire, TTL court).
    #[serde(with = "ms_required")]
    pub expires_at: UtcDateTime,
    /// Date de consommation (`None` = pas encore utilisé).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub used_at: Option<UtcDateTime>,
    /// Date de création.
    #[serde(with = "ms_required")]
    pub created_at: UtcDateTime,
}

impl DiscordLinkCode {
    /// `true` si le code peut encore être consommé à l'instant `now`.
    pub fn is_usable_at(&self, now: UtcDateTime) -> bool {
        self.used_at.is_none() && self.expires_at > now
    }
}

/// Overlay appairé (navigateur/desktop), authentifié par hash de token.
///
/// Le token en clair n'est **jamais** stocké : seul `token_hash` (SHA-256)
/// figure en base, et l'authentification WS v2 cherche la ligne non révoquée.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayClient {
    /// Clé primaire UUID.
    pub id: OverlayClientId,
    /// Guilde Discord à laquelle l'overlay est rattaché.
    pub guild_id: GuildId,
    /// Libellé lisible (ex. « Salon principal »).
    pub label: String,
    /// Hash SHA-256 du token d'appairage (unique).
    pub token_hash: String,
    /// Mode de session (normal ou invité lecture seule).
    pub session_mode: OverlaySessionMode,
    /// Nom d'auteur par défaut proposé à l'overlay.
    pub default_author_name: Option<String>,
    /// Avatar d'auteur par défaut proposé à l'overlay.
    pub default_author_image: Option<String>,
    /// Auteur Discord du pairing, si connu.
    pub created_by_discord_user_id: Option<String>,
    /// Empreinte serveur de l'identifiant d'appareil fourni à l'appairage.
    ///
    /// Le brut n'est jamais stocké ni journalisé : seule cette empreinte
    /// SHA-256, salée par la guilde, permet de refuser le retour d'un appareil
    /// banni. Absente pour les clients antérieurs au champ `deviceId`.
    ///
    /// Toujours omise des réponses : aucune API ne doit exposer `deviceHash`.
    #[serde(default, skip_serializing)]
    pub device_hash: Option<String>,
    /// Dernier heartbeat applicatif.
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<UtcDateTime>,
    /// Date de création.
    #[serde(with = "ms_required")]
    pub created_at: UtcDateTime,
    /// Date de révocation (`None` = actif).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<UtcDateTime>,
}

impl OverlayClient {
    /// `true` si le client n'est pas révoqué.
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// Client d'ingestion (extension navigateur), authentifié par hash de token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestClient {
    /// Clé primaire UUID.
    pub id: IngestClientId,
    /// Guilde Discord cible.
    pub guild_id: GuildId,
    /// Libellé lisible (ex. « Chrome maison »).
    pub label: String,
    /// Hash SHA-256 du token d'appairage (unique).
    pub token_hash: String,
    /// Nom d'auteur par défaut (obligatoire côté ingestion).
    pub default_author_name: String,
    /// Avatar d'auteur par défaut.
    pub default_author_image: Option<String>,
    /// Auteur Discord du pairing.
    pub created_by_discord_user_id: String,
    /// Dernier heartbeat applicatif.
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<UtcDateTime>,
    /// Date de création.
    #[serde(with = "ms_required")]
    pub created_at: UtcDateTime,
    /// Date de révocation (`None` = actif).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<UtcDateTime>,
}

/// Durée d'inactivité maximale d'un client d'ingestion, en millisecondes.
///
/// Valeur : 180 jours calendaires (`180 * 24 * 60 * 60 * 1000`). Aucune
/// migration n'ajoute de colonne `expires_at` : l'expiration est calculée
/// depuis [`IngestClient::last_activity_at`] (`last_seen_at` ou, à défaut,
/// `created_at`). 180 jours laisse une large marge face aux raccourcis iOS et
/// aux extensions qui ne sont utilisés qu'à l'occasion, tout en bornant la
/// durée de vie d'un jeton oublié sur un ancien appareil. Un client expiré
/// doit être ré-appairé depuis la board ; aucun heartbeat ne le ressuscite.
pub const INGEST_CLIENT_IDLE_EXPIRY_MS: i64 = 180 * 24 * 60 * 60 * 1000;

impl IngestClient {
    /// `true` si le client n'est pas révoqué.
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }

    /// Dernière activité connue : le heartbeat si présent, sinon la création.
    pub fn last_activity_at(&self) -> UtcDateTime {
        self.last_seen_at.unwrap_or(self.created_at)
    }

    /// `true` si le client peut encore ingérer à l'instant `now`.
    ///
    /// Un client révoqué est toujours inutilisable. Sinon il est utilisable
    /// jusqu'à `TTL - 1` ms inclus depuis [`Self::last_activity_at`] et expiré
    /// à `TTL` exactement. Une horloge en arrière (`now` avant l'activité)
    /// ne fait pas expirer le client.
    pub fn is_usable_at(&self, now: UtcDateTime) -> bool {
        self.revoked_at.is_none()
            && crate::time::datetime_to_ms(now)
                - crate::time::datetime_to_ms(self.last_activity_at())
                < INGEST_CLIENT_IDLE_EXPIRY_MS
    }
}

/// Code d'appairage à usage unique.
///
/// `expires_at == None` = code « invité » permanent (legacy :
/// `clientCommand.ts` ne pose `expiresAt` qu'en mode normal). La consommation
/// est atomique côté `lc-db` : non consommé **et** non expiré.
///
/// L'identifiant typé associé est [`PairingCodeValue`] (ré-exporté
/// `lc_domain::PairingCode`) : l'entité porte le suffixe `Entry` pour éviter
/// la collision de noms au niveau du crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCodeEntry {
    /// Code lisible (clé primaire, MAJUSCULES).
    pub code: PairingCodeValue,
    /// Guilde Discord cible.
    pub guild_id: GuildId,
    /// Mode accordé au futur client.
    pub mode: PairingMode,
    /// `true` si le code accorde le rôle LiveChat `admin` à l'appairage.
    ///
    /// Figé à la consommation : un membre admin issu d'un code garde ce rôle,
    /// même si les permissions Discord de son auteur changent ensuite.
    pub grants_admin: bool,
    /// Auteur Discord créateur (obligatoire).
    pub created_by_discord_user_id: String,
    /// Nom d'auteur affiché par l'overlay après appairage.
    pub author_name: Option<String>,
    /// Avatar d'auteur affiché par l'overlay après appairage.
    pub author_image: Option<String>,
    /// Expiration (`None` = permanent/invité).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<UtcDateTime>,
    /// Date de consommation (`None` = pas encore utilisé).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub used_at: Option<UtcDateTime>,
    /// Date de création.
    #[serde(with = "ms_required")]
    pub created_at: UtcDateTime,
}

impl PairingCodeEntry {
    /// `true` si le code peut encore être consommé à l'instant `now`.
    pub fn is_usable_at(&self, now: UtcDateTime) -> bool {
        self.used_at.is_none()
            && match self.expires_at {
                Some(expires_at) => expires_at > now,
                None => true,
            }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::INGEST_CLIENT_IDLE_EXPIRY_MS;
    use super::{IngestClient, MediaAsset, OverlayClient, PairingCodeEntry};
    use crate::enums::{MediaKind, MediaStatus, OverlaySessionMode, PairingMode};
    use crate::ids::{
        GuildId, IngestClientId, MediaAssetId, OverlayClientId, PairingCode as PairingCodeId,
    };
    use crate::time::ms_to_datetime;

    fn date(ms: i64) -> time::UtcDateTime {
        ms_to_datetime(ms).unwrap()
    }

    fn sample_asset() -> MediaAsset {
        MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: "abc123".to_owned(),
            source_url: "https://example.test/clip.mp4".to_owned(),
            source_title: Some("Un clip de test".to_owned()),
            kind: MediaKind::Video,
            mime: "video/mp4".to_owned(),
            duration_sec: Some(12),
            width: Some(1920),
            height: Some(1080),
            is_vertical: false,
            storage_path: "media/abc123.mp4".to_owned(),
            size_bytes: 4_096,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: date(1_700_000_000_000),
            expires_at: date(1_700_000_600_000),
            created_at: date(1_700_000_000_000),
        }
    }

    #[test]
    fn media_asset_json_roundtrip_ms_et_camel_case() {
        let asset = sample_asset();
        let json = serde_json::to_value(&asset).unwrap();

        assert_eq!(json["id"], asset.id.to_string());
        assert_eq!(json["sourceHash"], "abc123");
        assert_eq!(json["sourceTitle"], "Un clip de test");
        assert_eq!(json["isVertical"], false);
        assert_eq!(json["expiresAt"], 1_700_000_600_000_i64);
        assert_eq!(json["kind"], "video");

        let decoded: MediaAsset = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, asset);
    }

    #[test]
    fn media_asset_expiration() {
        let asset = sample_asset();
        assert!(asset.is_ready());
        assert!(!asset.is_expired_at(date(1_700_000_599_999)));
        assert!(asset.is_expired_at(date(1_700_000_600_000)));
    }

    #[test]
    fn overlay_client_never_serializes_its_device_fingerprint() {
        let client = OverlayClient {
            id: OverlayClientId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").unwrap(),
            label: "PC".to_owned(),
            token_hash: "hash".to_owned(),
            session_mode: OverlaySessionMode::Normal,
            default_author_name: None,
            default_author_image: None,
            created_by_discord_user_id: None,
            device_hash: Some("f".repeat(64)),
            last_seen_at: None,
            created_at: date(1_700_000_000_000),
            revoked_at: None,
        };

        let json = serde_json::to_value(&client).unwrap();
        assert!(
            json.get("deviceHash").is_none(),
            "aucune réponse ne doit exposer deviceHash : {json}"
        );

        // Un JSON sans le champ (client antérieur au `deviceId`) se relit.
        let decoded: OverlayClient = serde_json::from_value(json).unwrap();
        assert!(decoded.device_hash.is_none());
    }

    #[test]
    fn guild_invite_usable() {
        use super::GuildInvite;
        use crate::enums::GuildRole;

        let mut invite = GuildInvite {
            code: PairingCodeId::parse("K7QXM2PD").unwrap(),
            guild_id: GuildId::parse("123456789012345678").unwrap(),
            role: GuildRole::Member,
            member_id: None,
            max_uses: Some(2),
            use_count: 1,
            created_by_member_id: None,
            expires_at: Some(date(1_000)),
            revoked_at: None,
            created_at: date(0),
        };
        assert!(invite.is_usable_at(date(999)));
        assert!(!invite.is_usable_at(date(1_000)));
        invite.use_count = 2;
        assert!(!invite.is_usable_at(date(999)));
        invite.max_uses = None;
        invite.expires_at = None;
        assert!(invite.is_usable_at(date(999)));
        invite.revoked_at = Some(date(500));
        assert!(!invite.is_usable_at(date(999)));
    }

    fn sample_ingest_client() -> IngestClient {
        IngestClient {
            id: IngestClientId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").unwrap(),
            label: "Extension".to_owned(),
            token_hash: "hash".to_owned(),
            default_author_name: "LiveChat Extension".to_owned(),
            default_author_image: None,
            created_by_discord_user_id: "42".to_owned(),
            last_seen_at: None,
            created_at: date(1_700_000_000_000),
            revoked_at: None,
        }
    }

    #[test]
    fn ingest_client_idle_expiry_boundaries() {
        let client = sample_ingest_client();
        // Pas d'activité : `created_at` sert de référence.
        assert_eq!(client.last_activity_at(), client.created_at);
        let created = crate::time::datetime_to_ms(client.created_at);
        let fresh = date(created + INGEST_CLIENT_IDLE_EXPIRY_MS - 1);
        let expired = date(created + INGEST_CLIENT_IDLE_EXPIRY_MS);
        assert!(client.is_usable_at(fresh), "TTL-1 ms : encore utilisable");
        assert!(!client.is_usable_at(expired), "TTL exact : expiré");
        assert!(!client.is_usable_at(date(created + INGEST_CLIENT_IDLE_EXPIRY_MS + 1)));
        assert!(client.is_usable_at(client.created_at), "client frais");
        // Une horloge en arrière ne doit jamais expirer le client.
        assert!(client.is_usable_at(date(created - 1)));
    }

    #[test]
    fn ingest_client_last_seen_takes_precedence_and_revocation_wins() {
        let mut client = sample_ingest_client();
        let created = crate::time::datetime_to_ms(client.created_at);
        let later = created + INGEST_CLIENT_IDLE_EXPIRY_MS / 2;
        client.last_seen_at = Some(date(later));
        assert_eq!(client.last_activity_at(), date(later));
        // `created_at` seul serait déjà expiré : `last_seen_at` fait foi.
        assert!(client.is_usable_at(date(created + INGEST_CLIENT_IDLE_EXPIRY_MS - 1)));
        assert!(!client.is_usable_at(date(later + INGEST_CLIENT_IDLE_EXPIRY_MS)));

        client.revoked_at = Some(date(created));
        assert!(!client.is_usable_at(date(later)), "révoqué même frais");
    }

    #[test]
    fn pairing_code_usable() {
        let code = PairingCodeEntry {
            code: PairingCodeId::parse("AB12CD").unwrap(),
            guild_id: GuildId::parse("123456789012345678").unwrap(),
            mode: PairingMode::Normal,
            grants_admin: false,
            created_by_discord_user_id: "42".to_owned(),
            author_name: None,
            author_image: None,
            expires_at: Some(date(1_000)),
            used_at: None,
            created_at: date(0),
        };
        assert!(code.is_usable_at(date(999)));
        assert!(!code.is_usable_at(date(1_000)));
    }
}
