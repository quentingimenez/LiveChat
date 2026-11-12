//! Entités du domaine, alignées sur le contrat `docs/rust/schema-v3.sql`.
//!
//! - noms de champs : `snake_case` comme en base (pas de renommage de colonne
//!   caché dans `lc-db`) ;
//! - dates : [`chrono::DateTime<Utc>`] en mémoire, sérialisées en millisecondes
//!   epoch UTF lorsque l'entité est exposée en JSON (`camelCase`) ;
//! - booléens : `bool` Rust, stockés `INTEGER 0/1` avec `CHECK` côté base.
//!
//! Aucune entité ne contient de logique de persistance : les repositories
//! `lc-db` convertissent `row <-> entité`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::enums::{GuildRole, JobStatus, MediaKind, MediaStatus, OverlaySessionMode, PairingMode};
use crate::ids::{
    GuildId, GuildMemberId, IngestClientId, MediaAssetId, MemeBoardItemId, OverlayClientId,
    PairingCode as PairingCodeValue, PlaybackJobId,
};

/// Module serde pour les dates obligatoires en ms epoch UTC.
mod ms_required {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serializer::serialize_i64(serializer, value.timestamp_millis())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = i64::deserialize(deserializer)?;
        DateTime::from_timestamp_millis(millis)
            .ok_or_else(|| serde::de::Error::custom(format!("horodatage hors plage : {millis} ms")))
    }
}

/// Module serde pour les dates optionnelles en ms epoch UTC (`null` = absent).
mod ms_optional {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(date) => Serializer::serialize_some(serializer, &date.timestamp_millis()),
            None => Serializer::serialize_none(serializer),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = Option::<i64>::deserialize(deserializer)?;
        match millis {
            Some(millis) => DateTime::from_timestamp_millis(millis)
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
    pub last_accessed_at: DateTime<Utc>,
    /// Date d'expiration du cache.
    #[serde(with = "ms_required")]
    pub expires_at: DateTime<Utc>,
    /// Date de création de la ligne.
    #[serde(with = "ms_required")]
    pub created_at: DateTime<Utc>,
}

impl MediaAsset {
    /// `true` si le média est prêt à être servi.
    pub fn is_ready(&self) -> bool {
        self.status == MediaStatus::Ready
    }

    /// `true` si le média est expiré à l'instant `now`.
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

/// Élément du meme board d'une guilde.
///
/// Contraintes du schéma : `UNIQUE (guild_id, media_asset_id)` et
/// `ON DELETE CASCADE` depuis `media_assets` (purger un média supprime les
/// entrées de board qui le référencent).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemeBoardItem {
    /// Clé primaire UUID.
    pub id: MemeBoardItemId,
    /// Guilde Discord propriétaire.
    pub guild_id: GuildId,
    /// Média associé (aucun doublon par guilde).
    pub media_asset_id: MediaAssetId,
    /// Titre affiché sur le board.
    pub title: Option<String>,
    /// Texte/message associé au mème.
    pub message: Option<String>,
    /// Auteur Discord (snowflake), si l'ajout vient de Discord.
    pub created_by_discord_user_id: Option<String>,
    /// Nom d'affichage de l'auteur, figé à la création.
    pub created_by_name: Option<String>,
    /// Date d'ajout au board.
    #[serde(with = "ms_required")]
    pub created_at: DateTime<Utc>,
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
    /// Texte affiché pendant la lecture (mème textuel).
    pub text: Option<String>,
    /// Afficher le texte en plus du média.
    pub show_text: bool,
    /// Nom d'auteur affiché par l'overlay.
    pub author_name: Option<String>,
    /// URL de l'avatar d'auteur affiché.
    pub author_image: Option<String>,
    /// Durée de lecture demandée (secondes).
    pub duration_sec: i64,
    /// Priorité (plus grand = plus prioritaire).
    pub priority: i64,
    /// État courant.
    pub status: JobStatus,
    /// Job interrompu par celui-ci : ce job reprend après lui (legacy
    /// `resumesAfterJobId`).
    pub resumes_after_job_id: Option<PlaybackJobId>,
    /// Offset de reprise en secondes (`awaiting` avant reprise).
    pub resume_offset_sec: i64,
    /// Snapshot du temps restant (ms) au moment de l'interruption.
    pub remaining_ms_snapshot: Option<i64>,
    /// Dernier `playback.state` reçu (`playing`/`paused`).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub last_playback_state_at: Option<DateTime<Utc>>,
    /// Date de soumission (ordre FIFO secondaire).
    #[serde(with = "ms_required")]
    pub submission_date: DateTime<Utc>,
    /// Date d'exécution planifiée par le scheduler (legacy `executionDate`).
    #[serde(with = "ms_required")]
    pub execution_date: DateTime<Utc>,
    /// Date à partir de laquelle le job est éligible au claim.
    #[serde(with = "ms_required")]
    pub scheduled_at: DateTime<Utc>,
    /// Date de passage effectif en `Playing`.
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// Date de passage en état terminal.
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

impl PlaybackJob {
    /// `true` si l'état est terminal (`Done`/`Failed`).
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// `true` si le job est éligible au claim à l'instant `now`
    /// (état `Pending` et `scheduled_at` atteint).
    pub fn is_claimable_at(&self, now: DateTime<Utc>) -> bool {
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
    pub busy_until: Option<DateTime<Utc>>,
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
    /// Les invités peuvent consulter le meme board.
    pub guests_can_view_board: bool,
    /// Date de création (`None` pour une guilde antérieure à la migration 0003).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
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
    pub created_at: DateTime<Utc>,
    /// Date d'exclusion ou de départ (`None` = actif).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub removed_at: Option<DateTime<Utc>>,
}

impl GuildMember {
    /// `true` si le membre n'a ni quitté la guilde ni été exclu.
    pub fn is_active(&self) -> bool {
        self.removed_at.is_none()
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
    pub expires_at: Option<DateTime<Utc>>,
    /// Date de révocation (`None` = non révoquée).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    /// Date de création.
    #[serde(with = "ms_required")]
    pub created_at: DateTime<Utc>,
}

impl GuildInvite {
    /// `true` si l'invitation peut encore être utilisée à l'instant `now`.
    pub fn is_usable_at(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none()
            && self.expires_at.is_none_or(|expires_at| expires_at > now)
            && self
                .max_uses
                .is_none_or(|max_uses| self.use_count < max_uses)
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
    /// Dernier heartbeat applicatif.
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Date de création.
    #[serde(with = "ms_required")]
    pub created_at: DateTime<Utc>,
    /// Date de révocation (`None` = actif).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
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
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Date de création.
    #[serde(with = "ms_required")]
    pub created_at: DateTime<Utc>,
    /// Date de révocation (`None` = actif).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
}

impl IngestClient {
    /// `true` si le client n'est pas révoqué.
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
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
    /// Auteur Discord créateur (obligatoire).
    pub created_by_discord_user_id: String,
    /// Nom d'auteur affiché par l'overlay après appairage.
    pub author_name: Option<String>,
    /// Avatar d'auteur affiché par l'overlay après appairage.
    pub author_image: Option<String>,
    /// Expiration (`None` = permanent/invité).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Date de consommation (`None` = pas encore utilisé).
    #[serde(with = "ms_optional", default, skip_serializing_if = "Option::is_none")]
    pub used_at: Option<DateTime<Utc>>,
    /// Date de création.
    #[serde(with = "ms_required")]
    pub created_at: DateTime<Utc>,
}

impl PairingCodeEntry {
    /// `true` si le code peut encore être consommé à l'instant `now`.
    pub fn is_usable_at(&self, now: DateTime<Utc>) -> bool {
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
    use super::{MediaAsset, PairingCodeEntry};
    use crate::enums::{MediaKind, MediaStatus, PairingMode};
    use crate::ids::{GuildId, MediaAssetId, PairingCode as PairingCodeId};
    use crate::time::ms_to_datetime;

    fn date(ms: i64) -> chrono::DateTime<chrono::Utc> {
        ms_to_datetime(ms).unwrap()
    }

    fn sample_asset() -> MediaAsset {
        MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: "abc123".to_owned(),
            source_url: "https://example.test/clip.mp4".to_owned(),
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

    #[test]
    fn pairing_code_usable() {
        let code = PairingCodeEntry {
            code: PairingCodeId::parse("AB12CD").unwrap(),
            guild_id: GuildId::parse("123456789012345678").unwrap(),
            mode: PairingMode::Normal,
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
