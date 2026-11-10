//! Protocole WebSocket JSON v2 de LiveChat.
//!
//! Spécification gelée : `docs/rust/protocol-v2.md` (FROZEN rev. 2 pour
//! l'implémentation, datée 2026-11-09). Ce crate fournit l'enveloppe JSON, les types de messages
//! client→serveur et serveur→client, les constantes du protocole et les helpers
//! associés. Les fixtures golden de référence vivent dans `tests/fixtures/` et
//! sont vérifiées par `tests/fixtures_roundtrip.rs`.
#![forbid(unsafe_code)]

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// Version du protocole, présente dans chaque enveloppe (`v`).
pub const PROTOCOL_VERSION: u8 = 2;

/// Chemin du WebSocket natif v2.
pub const WS_PATH: &str = "/ws";

/// Chemin historique socket.io : toute requête doit être rejetée en 426.
pub const LEGACY_SOCKET_IO_PATH: &str = "/socket.io/";

/// Statut HTTP renvoyé aux clients socket.io obsolètes.
pub const HTTP_UPGRADE_REQUIRED: u16 = 426;

/// Délai maximal entre l'ouverture du WebSocket et la trame `hello`.
pub const HELLO_TIMEOUT_MS: u64 = 5_000;

/// Taille maximale d'une trame texte (64 Kio) ; au-delà : close 4408.
pub const MAX_FRAME_BYTES: usize = 65_536;

/// Nombre maximal de messages client par fenêtre d'une seconde.
pub const MAX_CLIENT_MESSAGES_PER_SECOND: u32 = 20;

/// Intervalle minimal entre deux `playback.state` (les excédents sont ignorés).
pub const PLAYBACK_STATE_MIN_INTERVAL_MS: u64 = 1_000;

/// Intervalle des `heartbeat` applicatifs v1 (conservé pour la parité).
pub const HEARTBEAT_INTERVAL_MS: u64 = 15_000;

/// Intervalle des ping WebSocket du serveur.
pub const PING_INTERVAL_MS: u64 = 15_000;

/// Délai sans pong avant fermeture 1001 `heartbeat_timeout`.
pub const PONG_TIMEOUT_MS: u64 = 45_000;

/// Durée de vie **plancher** des URLs média signées, en secondes.
pub const MEDIA_URL_TTL_SEC: u64 = 300;

/// Marge ajoutée à la durée d'un média pour calculer `exp` (secondes).
pub const MEDIA_URL_EXPIRY_MARGIN_SEC: u64 = 120;

/// Borne haute du TTL des URLs média signées (une heure).
pub const MEDIA_URL_TTL_MAX_SEC: u64 = 3_600;

/// Version d'overlay minimale par défaut (`MIN_OVERLAY_VERSION`), en semver simple.
pub const DEFAULT_MIN_OVERLAY_VERSION: &str = "3.0.0";

/// Intervalle minimal entre deux demandes de pairs.
pub const PEERS_REQUEST_MIN_INTERVAL_MS: u64 = 2_000;

/// Borne haute de `remainingMs` acceptée (24 heures).
pub const MAX_PLAYBACK_REMAINING_MS: u64 = 86_400_000;

/// Longueur maximale du texte d'un mème déclenché depuis l'overlay.
pub const MAX_MEME_TEXT_CHARS: usize = 500;

/// Codes de fermeture WebSocket v2.
pub mod close_code {
    /// Fermeture normale demandée par le client ou le serveur.
    pub const NORMAL: u16 = 1000;
    /// Arrêt de service ou timeout de heartbeat.
    pub const GOING_AWAY: u16 = 1001;
    /// Trame non conforme au protocole.
    pub const PROTOCOL_ERROR: u16 = 1002;
    /// Type de trame non supporté/état de lecture invalide.
    pub const UNSUPPORTED_FRAME: u16 = 1003;
    /// Erreur interne du serveur.
    pub const INTERNAL_ERROR: u16 = 1011;
    /// Authentification manquante, invalide ou `hello` absent.
    pub const UNAUTHORIZED: u16 = 4401;
    /// Client révoqué par l'administration.
    pub const REVOKED: u16 = 4402;
    /// Version de protocole obsolète.
    pub const OUTDATED_PROTOCOL: u16 = 4403;
    /// Trame dépassant `MAX_FRAME_BYTES`.
    pub const FRAME_TOO_LARGE: u16 = 4408;
    /// Débit client dépassant `MAX_CLIENT_MESSAGES_PER_SECOND`.
    pub const RATE_LIMITED: u16 = 4409;
}

/// Motifs de fermeture (chaîne `reason` des close frames v2).
pub mod close_reason {
    /// Arrêt propre demandé par le client.
    pub const CLIENT_SHUTDOWN: &str = "client_shutdown";
    /// Arrêt du serveur.
    pub const SERVER_SHUTDOWN: &str = "server_shutdown";
    /// Aucun pong reçu dans `PONG_TIMEOUT_MS`.
    pub const HEARTBEAT_TIMEOUT: &str = "heartbeat_timeout";
    /// Trame ou enveloppe non conforme.
    pub const PROTOCOL_ERROR: &str = "protocol_error";
    /// `playback.state` incohérent.
    pub const BAD_PLAYBACK_STATE: &str = "bad_playback_state";
    /// Type de trame non supporté.
    pub const UNSUPPORTED_FRAME: &str = "unsupported_frame";
    /// Erreur interne.
    pub const INTERNAL_ERROR: &str = "internal_error";
    /// Jeton absent de la trame `hello`.
    pub const MISSING_TOKEN: &str = "missing_token";
    /// Jeton inconnu (hash absent de `overlay_clients`).
    pub const INVALID_TOKEN: &str = "invalid_token";
    /// Aucun `hello` reçu dans `HELLO_TIMEOUT_MS`.
    pub const AUTH_TIMEOUT: &str = "auth_timeout";
    /// `hello` requis avant toute autre trame.
    pub const HELLO_REQUIRED: &str = "hello_required";
    /// Client révoqué.
    pub const REVOKED: &str = "revoked";
    /// Client d'une version d'overlay trop ancienne.
    pub const OUTDATED_PROTOCOL: &str = "outdated_protocol";
    /// Trame trop volumineuse.
    pub const FRAME_TOO_LARGE: &str = "frame_too_large";
    /// Débit trop élevé.
    pub const RATE_LIMITED: &str = "rate_limited";
}

/// Noms des messages (valeur du champ `type` de l'enveloppe).
pub mod msg_type {
    /// C→S : poignée de main initiale, sous 5 s.
    pub const HELLO: &str = "hello";
    /// C→S : heartbeat applicatif, toutes les 15 s.
    pub const HEARTBEAT: &str = "heartbeat";
    /// C→S : état de lecture, au plus 1/s.
    pub const PLAYBACK_STATE: &str = "playback.state";
    /// C→S : fin de lecture.
    pub const PLAYBACK_ENDED: &str = "playback.ended";
    /// C→S : erreur de rendu signalée par l'overlay.
    pub const ERROR: &str = "error";
    /// C→S : demande d'arrêt manuel de la lecture.
    pub const STOP_REQUEST: &str = "stop.request";
    /// C→S : déclenchement d'un mème.
    pub const MEME_TRIGGER: &str = "meme.trigger";
    /// C→S : demande de la liste des pairs.
    pub const PEERS_REQUEST: &str = "peers.request";
    /// S→C : acceptation du `hello`.
    pub const WELCOME: &str = "welcome";
    /// S→C : réglages de la guilde.
    pub const SETTINGS: &str = "settings";
    /// S→C : liste des overlays connectés.
    pub const PEERS: &str = "peers";
    /// S→C : ordre de lecture d'un job.
    pub const PLAY: &str = "play";
    /// S→C : arrêt de lecture (manuel, préemption, révocation, purge).
    pub const STOP: &str = "stop";
    /// S→C : acquittement d'un `stop.request` ou `meme.trigger`.
    pub const ACK: &str = "ack";
}

/// Indique si une trame dépasse la limite autorisée.
#[must_use]
pub fn is_frame_too_large(len: usize) -> bool {
    len > MAX_FRAME_BYTES
}

/// Construit l'entrée signée HMAC-SHA256 d'une URL média : `assetId|clientId|exp`.
///
/// `exp_sec` est un horodatage Unix **en secondes** ; la signature s'écrit en
/// hexadécimal minuscule côté serveur (cf. doc §8).
#[must_use]
pub fn media_signature_input(asset_id: &str, client_id: &str, exp_sec: u64) -> String {
    format!("{asset_id}|{client_id}|{exp_sec}")
}

/// Calcule le TTL d'une URL média signée, en secondes.
///
/// Décision gelée (doc §8.1, §15.1) : une image (ou une durée inconnue) reçoit
/// `MEDIA_URL_TTL_SEC` ; une vidéo/audio dont la durée est connue reçoit
/// `clamp(durationSec + MEDIA_URL_EXPIRY_MARGIN_SEC, MEDIA_URL_TTL_SEC,
/// MEDIA_URL_TTL_MAX_SEC)`.
#[must_use]
pub fn media_url_ttl_sec(kind: MediaKind, duration_sec: Option<u64>) -> u64 {
    match kind {
        MediaKind::Image => MEDIA_URL_TTL_SEC,
        MediaKind::Audio | MediaKind::Video => duration_sec.map_or(MEDIA_URL_TTL_SEC, |duration| {
            duration
                .saturating_add(MEDIA_URL_EXPIRY_MARGIN_SEC)
                .clamp(MEDIA_URL_TTL_SEC, MEDIA_URL_TTL_MAX_SEC)
        }),
    }
}

/// Construit la `reason` JSON d'une fermeture `4403` : `{"minVersion":"x.y.z"}`.
#[must_use]
pub fn outdated_protocol_reason(min_version: &str) -> String {
    format!("{{\"minVersion\":\"{min_version}\"}}")
}

/// Analyse une version « semver simple » `major.minor.patch`.
///
/// Les suffixes de pré-version (`-beta.1`) et de build (`+sha`) sont ignorés ;
/// les trois composantes doivent être numériques, sinon `None`.
#[must_use]
pub fn parse_simple_version(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Indique si `app_version` est strictement inférieure à `min_version`.
///
/// Règle gelée (doc §1.6) : comparaison lexicographique des triplets
/// `(major, minor, patch)` ; une version illisible est considérée obsolète
/// (`true`, refus par sécurité).
#[must_use]
pub fn is_version_below(app_version: &str, min_version: &str) -> bool {
    match (
        parse_simple_version(app_version),
        parse_simple_version(min_version),
    ) {
        (Some(app), Some(min)) => app < min,
        _ => true,
    }
}

/// Enveloppe JSON v2 : `{ "v": 2, "id": "...", "ts": ..., "type": "...", "payload": {...} }`.
///
/// `M` est [`ClientMessage`] ou [`ServerMessage`] ; la paire `type`/`payload` est
/// produite par l'enum (représentation adjacente) puis aplatie au même niveau que
/// `v`, `id` et `ts`.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope<M> {
    /// Version du protocole, toujours `PROTOCOL_VERSION` sur le fil.
    pub v: u8,
    /// Identifiant opaque de la trame (non vide, unique par connexion).
    pub id: String,
    /// Horodatage d'émission, en millisecondes Unix.
    pub ts: i64,
    /// Message transporté.
    pub message: M,
}

impl<M> Envelope<M> {
    /// Construit une enveloppe v2 datée.
    #[must_use]
    pub fn new(id: impl Into<String>, ts: i64, message: M) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            ts,
            message,
        }
    }
}

impl<M> Serialize for Envelope<M>
where
    M: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let message = serde_json::to_value(&self.message).map_err(serde::ser::Error::custom)?;
        let Value::Object(mut payload_fields) = message else {
            return Err(serde::ser::Error::custom(
                "un message de protocole doit se sérialiser en objet JSON",
            ));
        };
        let mut out = Map::new();
        out.insert("v".to_owned(), Value::from(self.v));
        out.insert("id".to_owned(), Value::from(self.id.clone()));
        out.insert("ts".to_owned(), Value::from(self.ts));
        for (key, value) in std::mem::take(&mut payload_fields) {
            out.insert(key, value);
        }
        Value::Object(out).serialize(serializer)
    }
}

impl<'de, M> Deserialize<'de> for Envelope<M>
where
    M: DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(mut fields) = value else {
            return Err(serde::de::Error::custom(
                "une enveloppe de protocole doit être un objet JSON",
            ));
        };
        let v = fields
            .remove("v")
            .ok_or_else(|| serde::de::Error::custom("champ `v` manquant dans l'enveloppe"))?;
        let id = fields
            .remove("id")
            .ok_or_else(|| serde::de::Error::custom("champ `id` manquant dans l'enveloppe"))?;
        let ts = fields
            .remove("ts")
            .ok_or_else(|| serde::de::Error::custom("champ `ts` manquant dans l'enveloppe"))?;
        let v = serde_json::from_value(v).map_err(serde::de::Error::custom)?;
        let id = serde_json::from_value(id).map_err(serde::de::Error::custom)?;
        let ts = serde_json::from_value(ts).map_err(serde::de::Error::custom)?;
        let message =
            serde_json::from_value(Value::Object(fields)).map_err(serde::de::Error::custom)?;
        Ok(Self { v, id, ts, message })
    }
}

/// Enveloppe d'un message client→serveur.
pub type ClientEnvelope = Envelope<ClientMessage>;

/// Enveloppe d'un message serveur→client.
pub type ServerEnvelope = Envelope<ServerMessage>;

/// Message client→serveur (v2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ClientMessage {
    /// Poignée de main d'authentification (sous 5 s).
    #[serde(rename = "hello")]
    Hello(HelloPayload),
    /// Heartbeat applicatif.
    #[serde(rename = "heartbeat")]
    Heartbeat(HeartbeatPayload),
    /// État de lecture courant.
    #[serde(rename = "playback.state")]
    PlaybackState(PlaybackStatePayload),
    /// Fin de lecture.
    #[serde(rename = "playback.ended")]
    PlaybackEnded(PlaybackEndedPayload),
    /// Erreur de rendu signalée par l'overlay.
    #[serde(rename = "error")]
    Error(ClientErrorPayload),
    /// Demande d'arrêt manuel.
    #[serde(rename = "stop.request")]
    StopRequest(StopRequestPayload),
    /// Déclenchement d'un mème.
    #[serde(rename = "meme.trigger")]
    MemeTrigger(MemeTriggerPayload),
    /// Demande de la liste des pairs.
    #[serde(rename = "peers.request")]
    PeersRequest(PeersRequestPayload),
}

impl ClientMessage {
    /// Nom du message (champ `type` de l'enveloppe).
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Hello(_) => msg_type::HELLO,
            Self::Heartbeat(_) => msg_type::HEARTBEAT,
            Self::PlaybackState(_) => msg_type::PLAYBACK_STATE,
            Self::PlaybackEnded(_) => msg_type::PLAYBACK_ENDED,
            Self::Error(_) => msg_type::ERROR,
            Self::StopRequest(_) => msg_type::STOP_REQUEST,
            Self::MemeTrigger(_) => msg_type::MEME_TRIGGER,
            Self::PeersRequest(_) => msg_type::PEERS_REQUEST,
        }
    }
}

/// Message serveur→client (v2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerMessage {
    /// Acceptation du `hello`.
    #[serde(rename = "welcome")]
    Welcome(WelcomePayload),
    /// Réglages de la guilde.
    #[serde(rename = "settings")]
    Settings(SettingsPayload),
    /// Liste des overlays connectés.
    #[serde(rename = "peers")]
    Peers(PeersPayload),
    /// Ordre de lecture.
    #[serde(rename = "play")]
    Play(PlayPayload),
    /// Arrêt de lecture.
    #[serde(rename = "stop")]
    Stop(StopPayload),
    /// Acquittement.
    #[serde(rename = "ack")]
    Ack(AckPayload),
    /// Erreur côté serveur.
    #[serde(rename = "error")]
    Error(ServerErrorPayload),
}

impl ServerMessage {
    /// Nom du message (champ `type` de l'enveloppe).
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Welcome(_) => msg_type::WELCOME,
            Self::Settings(_) => msg_type::SETTINGS,
            Self::Peers(_) => msg_type::PEERS,
            Self::Play(_) => msg_type::PLAY,
            Self::Stop(_) => msg_type::STOP,
            Self::Ack(_) => msg_type::ACK,
            Self::Error(_) => msg_type::ERROR,
        }
    }
}

/// Modes de session d'un overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// Session normale : lecture, mèmes et arrêt autorisés.
    Normal,
    /// Session invitée : mèmes et arrêt refusés (`ack` `read_only`).
    InviteReadOnly,
}

/// Type d'un média de la bibliothèque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    /// Image fixe.
    Image,
    /// Piste audio.
    Audio,
    /// Vidéo.
    Video,
}

/// État de lecture transmis par l'overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    /// Lecture en cours.
    Playing,
    /// Lecture en pause.
    Paused,
}

/// Raison de la fin de lecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackEndedReason {
    /// Média arrivé à son terme.
    Completed,
    /// Arrêt manuel.
    Manual,
    /// Erreur de rendu.
    Error,
}

/// Raison d'un ordre d'arrêt serveur→client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StopReason {
    /// Arrêt manuel demandé par un opérateur.
    Manual,
    /// Préemption par un job prioritaire (mème).
    Preempt,
    /// Client révoqué par l'administration.
    Revoked,
    /// Médias purgés par l'administration.
    Purge,
}

/// Origine d'un déclenchement de mème.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemeTrigger {
    /// Raccourci clavier configuré.
    Shortcut,
    /// Déclenchement depuis l'interface du board.
    Ui,
}

/// Origine déclarative d'un `stop.request` (informationnelle, défaut serveur `user`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StopRequestSource {
    /// Demande d'un utilisateur.
    User,
    /// Raccourci clavier global.
    Hotkey,
    /// Action d'administration.
    Admin,
}

/// Payload de `hello`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloPayload {
    /// Jeton d'appairage en clair (le serveur compare son SHA-256).
    pub token: String,
    /// Version de l'application overlay.
    pub app_version: String,
    /// Identifiant du client s'il est déjà connu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Mode de session demandé (diagnostic ; le serveur fait autorité).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_mode: Option<SessionMode>,
}

/// Payload de `heartbeat`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatPayload {
    /// Identifiant du client.
    pub client_id: String,
    /// Identifiant de la guilde.
    pub guild_id: String,
    /// Version de l'application overlay.
    pub app_version: String,
    /// Mode de session courant (diagnostic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_mode: Option<SessionMode>,
}

/// Payload de `playback.state`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStatePayload {
    /// Identifiant du job concerné.
    pub job_id: String,
    /// État courant (`playing` ou `paused`).
    pub state: PlaybackState,
    /// Temps restant estimé, en millisecondes (`null` si inconnu).
    pub remaining_ms: Option<u64>,
}

/// Payload de `playback.ended`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackEndedPayload {
    /// Identifiant du job terminé.
    pub job_id: String,
    /// Raison de la fin de lecture.
    pub reason: PlaybackEndedReason,
}

/// Payload de `error` client→serveur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientErrorPayload {
    /// Identifiant du job en erreur.
    pub job_id: String,
    /// Code d'erreur (ex. `render_failed`).
    pub code: String,
    /// Message lisible.
    pub message: String,
}

/// Payload de `stop.request`.
///
/// `jobId` absent/null = arrêt de tous les jobs de la guilde ; `source` est
/// purement informatif (le serveur applique `user` par défaut).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopRequestPayload {
    /// Job ciblé ; absent/null = tous les jobs de la guilde.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Origine déclarative de la demande.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<StopRequestSource>,
}

/// Payload de `meme.trigger`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemeTriggerPayload {
    /// Identifiant de l'item mème.
    pub item_id: String,
    /// Origine du déclenchement.
    pub trigger: MemeTrigger,
}

/// Payload de `peers.request` (vide).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeersRequestPayload {}

/// Configuration de guilde transmise dans `welcome`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeConfig {
    /// `showText` par défaut pour les jobs sans préférence.
    pub show_text_default: bool,
    /// Durée par défaut des médias, en secondes.
    pub default_media_time: u64,
    /// Durée maximale des médias (`null` = illimité).
    pub max_media_time: Option<u64>,
}

/// Payload de `welcome`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomePayload {
    /// Version du protocole acceptée par le serveur.
    pub protocol_version: u8,
    /// Identifiant de la guilde.
    pub guild_id: String,
    /// Identifiant canonique du client.
    pub client_id: String,
    /// Mode de session autoritatif.
    pub session_mode: SessionMode,
    /// Version sémantique du serveur (ex. `3.0.0`), toujours émise.
    pub server_version: String,
    /// Heure serveur (millisecondes Unix) pour recaler l'horloge.
    pub server_time: i64,
    /// Configuration par défaut de la guilde.
    pub config: WelcomeConfig,
    /// Job éventuellement en cours lors d'une reconnexion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_job: Option<CurrentJobPayload>,
}

/// Job en cours renvoyé dans `welcome` lors d'une reconnexion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentJobPayload {
    /// Payload `play` complet du job.
    #[serde(flatten)]
    pub play: PlayPayload,
    /// Horodatage de démarrage de la lecture (millisecondes Unix).
    pub started_at: i64,
    /// Temps restant estimé, en millisecondes.
    pub remaining_ms: u64,
}

/// Payload de `settings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPayload {
    /// Identifiant de la guilde.
    pub guild_id: String,
    /// Mode de session autoritatif.
    pub session_mode: SessionMode,
    /// `showText` par défaut.
    pub show_text_default: bool,
    /// Durée par défaut des médias, en secondes.
    pub default_media_time: u64,
    /// Durée maximale des médias (`null` = illimité).
    pub max_media_time: Option<u64>,
}

/// Informations sur un overlay connecté.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    /// Identifiant unique du client.
    pub client_id: String,
    /// Libellé affiché (auteur, appareil ou `unknown-device`).
    pub label: String,
}

/// Payload de `peers`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeersPayload {
    /// Identifiant de la guilde.
    pub guild_id: String,
    /// Pairs connectés sur la même guilde.
    pub peers: Vec<PeerInfo>,
}

/// Média d'un job `play`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPayload {
    /// Identifiant de l'asset média.
    pub asset_id: String,
    /// URL signée prête à l'emploi (TTL 300 s).
    pub url: String,
    /// Type MIME.
    pub mime: String,
    /// Nature du média.
    pub kind: MediaKind,
    /// Durée totale de l'asset (`null` si inconnue).
    pub duration_sec: Option<u64>,
    /// Média vertical (format mobile).
    pub is_vertical: bool,
    /// Offset de reprise dans l'asset (`null` si début).
    pub start_offset_sec: Option<u64>,
}

/// Texte affiché par un job `play`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPayload {
    /// Contenu affiché.
    pub value: String,
    /// Le texte doit-il être rendu.
    pub enabled: bool,
}

/// Auteur affiché par un job `play`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorPayload {
    /// Nom affiché.
    pub name: String,
    /// URL d'avatar (`null` si absente).
    pub image: Option<String>,
    /// L'auteur doit-il être rendu.
    pub enabled: bool,
}

/// Entrée vidéo enrichie d'une carte tweet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweetCardVideo {
    /// URL de la vidéo.
    pub url: String,
    /// Type MIME (`null` si inconnu).
    pub mime: Option<String>,
    /// Vidéo verticale (`null` si inconnu).
    pub is_vertical: Option<bool>,
    /// Statut source d'origine (`null` si inconnu).
    pub source_status_id: Option<String>,
    /// Durée en secondes (`null` si inconnue).
    pub duration_sec: Option<u64>,
}

/// Carte tweet attachée à un job `play`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweetCardPayload {
    /// URL du tweet.
    pub url: String,
    /// HTML d'embed fourni par le scraper.
    pub html: String,
    /// Nom de l'auteur du tweet.
    pub author_name: String,
    /// URL du profil de l'auteur (`null` si absente).
    pub author_url: Option<String>,
    /// Légende du tweet (`null` si absente).
    pub caption: Option<String>,
    /// Identifiant du statut courant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_status_id: Option<String>,
    /// URL de la vidéo extraite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_url: Option<String>,
    /// Type MIME de la vidéo extraite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_mime: Option<String>,
    /// Vidéo verticale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_is_vertical: Option<bool>,
    /// Vidéos disponibles dans le tweet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub videos: Option<Vec<TweetCardVideo>>,
    /// Champs inconnus conservés tels quels (tolérance §2 : un client récent
    /// qui renvoie une carte enrichie ne perd pas les clés qu'il ne connaît
    /// pas).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Payload de `play`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayPayload {
    /// Identifiant du job de lecture.
    pub job_id: String,
    /// Média à lire (`null` pour un job texte seul).
    pub media: Option<MediaPayload>,
    /// Texte overlay.
    pub text: TextPayload,
    /// Auteur affiché (masqué pour les cartes tweet).
    pub author: AuthorPayload,
    /// Carte tweet (`null` si absente).
    pub tweet_card: Option<TweetCardPayload>,
    /// Durée effective de lecture, en secondes.
    pub duration_sec: u64,
    /// Champs inconnus conservés tels quels (tolérance §2, ex. extensions
    /// futures de `play` ; une map vide n'émet aucune clé).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for PlayPayload {
    fn default() -> Self {
        Self {
            job_id: String::new(),
            media: None,
            text: TextPayload {
                value: String::new(),
                enabled: false,
            },
            author: AuthorPayload {
                name: String::new(),
                image: None,
                enabled: false,
            },
            tweet_card: None,
            duration_sec: 0,
            extra: serde_json::Map::new(),
        }
    }
}

/// Payload de `stop`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopPayload {
    /// Raison de l'arrêt.
    pub reason: StopReason,
    /// Job ciblé ; `null` signifie « tous les jobs de la guilde ».
    pub job_id: Option<String>,
}

/// Erreur détaillée d'un `ack` refusé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AckError {
    /// Code machine (ex. `read_only`).
    pub code: String,
    /// Message lisible.
    pub message: String,
}

/// Payload de `ack`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AckPayload {
    /// `id` de la trame acquittée.
    #[serde(rename = "ref")]
    pub ref_id: String,
    /// La demande a-t-elle été acceptée.
    pub ok: bool,
    /// Détail du refus (`ok: false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AckError>,
}

/// Payload de `error` serveur→client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerErrorPayload {
    /// Code machine (ex. `dispatch_failed`).
    pub code: String,
    /// Message lisible.
    pub message: String,
    /// Job concerné, si l'erreur s'y rattache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}
