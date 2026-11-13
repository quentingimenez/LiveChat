//! Configuration persistée de l'overlay.
//!
//! Le fichier de configuration est **forcé** sur `%APPDATA%\LiveChat\config.json`
//! (décision `docs/rust/overlay-bridge.md` §8.6) et non sur `app_config_dir()` de
//! Tauri (`%APPDATA%\com.livechat`) : c'est le même fichier que l'application
//! Electron 2.0.1, ce qui permet un rollback sans perte de configuration.
//!
//! Les normalisations reproduisent `overlay/src/main/services/config-service.ts`
//! (volume borné, booléens stricts, accélérateurs/dictionnaire nettoyés). Une
//! variable d'environnement `LIVECHAT_CONFIG_PATH` permet d'isoler les tests
//! d'intégration et la variante bêta.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Nom du dossier de configuration dans `%APPDATA%` (compatibilité Electron).
pub const CONFIG_DIR_NAME: &str = "LiveChat";
/// Nom du fichier de configuration (compatibilité Electron).
pub const CONFIG_FILE_NAME: &str = "config.json";
/// Suffixe de la sauvegarde de migration.
pub const CONFIG_BACKUP_SUFFIX: &str = ".bak";
/// Variable d'environnement prioritaire pour surcharger le chemin de config.
pub const CONFIG_PATH_ENV: &str = "LIVECHAT_CONFIG_PATH";

const DEFAULT_VOLUME: f64 = 1.0;

const KNOWN_KEYS: [&str; 21] = [
    "displayId",
    "displayIndex",
    "displayKey",
    "displayLabel",
    "volume",
    "enabled",
    "autoStart",
    "guestMode",
    "showText",
    "serverUrl",
    "clientToken",
    "clientId",
    "guildId",
    "authorName",
    "deviceName",
    "mediaBindings",
    "memberships",
    "profileName",
    "profileAvatar",
    "shortcuts",
    "layout",
];

/// Adhésion à une guilde : un jeton overlay par guilde rejointe.
///
/// L'overlay reste connecté à toutes ses guildes non muettes. Les champs
/// historiques de [`OverlayConfig`] (`serverUrl`, `clientToken`, `clientId`,
/// `guildId`, `guestMode`, `mediaBindings`) sont le **miroir de la guilde
/// active** (celle affichée dans le board) : le renderer d'affichage, le tray
/// et les raccourcis continuent de les lire tels quels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Membership {
    pub server_url: String,
    pub guild_id: String,
    pub guild_name: Option<String>,
    /// Discord CDN URL of the guild icon, when known.
    pub guild_icon_url: Option<String>,
    pub client_token: String,
    pub client_id: Option<String>,
    pub member_id: Option<String>,
    pub display_name: Option<String>,
    /// `admin`, `member` ou `guest` (valeur du serveur, informative : le
    /// serveur reste seul juge des droits).
    pub role: Option<String>,
    pub guest_mode: bool,
    /// Guilde coupée : aucune connexion, donc aucun média affiché.
    pub muted: bool,
    /// Guild-specific volume, multiplied by the general volume.
    pub volume: f64,
    /// Raccourcis conservés par guilde.
    pub media_bindings: MediaBindings,
}

impl Default for Membership {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            guild_id: String::new(),
            guild_name: None,
            guild_icon_url: None,
            client_token: String::new(),
            client_id: None,
            member_id: None,
            display_name: None,
            role: None,
            guest_mode: false,
            muted: false,
            volume: DEFAULT_VOLUME,
            media_bindings: MediaBindings::new(),
        }
    }
}

impl Membership {
    fn is_valid(&self) -> bool {
        !self.server_url.trim().is_empty()
            && !self.guild_id.trim().is_empty()
            && !self.client_token.trim().is_empty()
    }
}

/// Vue d'une adhésion pour l'interface : jamais de jeton.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MembershipView {
    pub server_url: String,
    pub guild_id: String,
    pub guild_name: Option<String>,
    pub guild_icon_url: Option<String>,
    pub client_id: Option<String>,
    pub member_id: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub guest_mode: bool,
    pub muted: bool,
    pub active: bool,
}

/// Dictionnaire accélérateur -> identifiant d'item du board média.
pub type MediaBindings = BTreeMap<String, String>;

/// Global shortcuts owned by the shell (not the per-media bindings).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShortcutSettings {
    /// Stop the current board media.
    pub stop: Option<String>,
    /// Toggle the general mute (same action as the tray entry).
    pub mute_all: Option<String>,
}

impl ShortcutSettings {
    fn normalize(&mut self) {
        self.stop = normalized_shortcut(self.stop.take());
        self.mute_all = normalized_shortcut(self.mute_all.take());
    }
}

fn normalized_shortcut(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Where a media sits in the display zone when it does not have its shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutAnchor {
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Smallest zone side, as a fraction of the screen side.
pub const MIN_ZONE_FRACTION: f64 = 0.05;

/// Zone fractions are kept to 1/10 000 (0.1 px on a 1000 px side).
const ZONE_FRACTION_SCALE: f64 = 10_000.0;

/// Rectangle of the screen the media frame (header, media, caption) must fit
/// in, as fractions of the selected screen: `0,0,1,1` is the whole screen.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LayoutZone {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for LayoutZone {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

impl LayoutZone {
    /// Keeps the zone on screen and at least [`MIN_ZONE_FRACTION`] wide and
    /// high; a non-finite value resets the whole zone.
    pub fn normalize(&mut self) {
        if ![self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite())
        {
            *self = Self::default();
            return;
        }
        self.width = round_fraction(self.width.clamp(MIN_ZONE_FRACTION, 1.0));
        self.height = round_fraction(self.height.clamp(MIN_ZONE_FRACTION, 1.0));
        self.x = round_fraction(self.x.clamp(0.0, 1.0 - self.width));
        self.y = round_fraction(self.y.clamp(0.0, 1.0 - self.height));
    }
}

fn round_fraction(value: f64) -> f64 {
    // Dividing an integer gives the nearest float (7000 / 10 000 == 0.7).
    (value * ZONE_FRACTION_SCALE).round() / ZONE_FRACTION_SCALE
}

/// Media frame layout, persisted in `OverlayConfig` and sent to the renderer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlayLayout {
    pub zone: LayoutZone,
    pub align: LayoutAnchor,
}

impl OverlayLayout {
    pub fn normalize(&mut self) {
        self.zone.normalize();
    }
}

/// Configuration complète persistée (16 champs legacy + champs inconnus préservés).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayConfig {
    pub display_id: Option<i64>,
    pub display_index: Option<i64>,
    pub display_key: Option<String>,
    pub display_label: Option<String>,
    pub volume: f64,
    pub enabled: bool,
    pub auto_start: bool,
    pub guest_mode: bool,
    pub show_text: bool,
    pub server_url: Option<String>,
    pub client_token: Option<String>,
    pub client_id: Option<String>,
    pub guild_id: Option<String>,
    pub author_name: Option<String>,
    pub device_name: Option<String>,
    pub media_bindings: MediaBindings,
    /// Guildes rejointes (la guilde active est aussi reflétée dans les champs
    /// historiques ci-dessus).
    pub memberships: Vec<Membership>,
    /// Pseudo du profil local, proposé à chaque nouvelle guilde.
    pub profile_name: Option<String>,
    /// Avatar du profil local.
    pub profile_avatar: Option<String>,
    /// Global stop / mute-all shortcuts.
    pub shortcuts: ShortcutSettings,
    /// Media frame placement sent to the renderer.
    pub layout: OverlayLayout,
    /// Champs écrits par une autre version (Electron) : conservés à l'écriture
    /// pour ne pas casser un rollback.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            display_id: None,
            display_index: None,
            display_key: None,
            display_label: None,
            volume: DEFAULT_VOLUME,
            enabled: true,
            auto_start: false,
            guest_mode: false,
            show_text: true,
            server_url: None,
            client_token: None,
            client_id: None,
            guild_id: None,
            author_name: None,
            device_name: None,
            media_bindings: MediaBindings::new(),
            memberships: Vec::new(),
            profile_name: None,
            profile_avatar: None,
            shortcuts: ShortcutSettings::default(),
            layout: OverlayLayout::default(),
            extra: BTreeMap::new(),
        }
    }
}

/// Renderer projection (legacy renderer fields plus layout).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayConfigView {
    pub server_url: Option<String>,
    pub client_token: Option<String>,
    pub guild_id: Option<String>,
    pub client_id: Option<String>,
    pub guest_mode: bool,
    pub show_text: bool,
    pub volume: f64,
    pub media_bindings: MediaBindings,
    pub layout: OverlayLayout,
}

impl OverlayConfig {
    /// Parse tolérant (les champs invalides retombent sur les défauts et sont
    /// signalés dans `warnings` au lieu de faire échouer tout le fichier).
    pub fn from_json_value(value: &Value) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut config = Self::default();
        let Some(map) = value.as_object() else {
            warnings.push("config.json ne contient pas un objet JSON".to_string());
            return (config, warnings);
        };

        config.display_id = int_field(map, "displayId", &mut warnings);
        config.display_index = int_field(map, "displayIndex", &mut warnings);
        config.display_key = string_field(map, "displayKey", &mut warnings);
        config.display_label = string_field(map, "displayLabel", &mut warnings);
        config.volume = volume_field(map, &mut warnings);
        config.enabled = bool_field(map, "enabled", true, &mut warnings);
        config.auto_start = map
            .get("autoStart")
            .is_some_and(|value| value == &Value::Bool(true));
        config.guest_mode = map
            .get("guestMode")
            .is_some_and(|value| value == &Value::Bool(true));
        config.show_text = bool_field(map, "showText", true, &mut warnings);
        config.server_url = string_field(map, "serverUrl", &mut warnings);
        config.client_token = string_field(map, "clientToken", &mut warnings);
        config.client_id = string_field(map, "clientId", &mut warnings);
        config.guild_id = string_field(map, "guildId", &mut warnings);
        config.author_name = string_field(map, "authorName", &mut warnings);
        config.device_name = string_field(map, "deviceName", &mut warnings);
        config.media_bindings = media_bindings_field(map, &mut warnings);
        config.profile_name = string_field(map, "profileName", &mut warnings);
        config.profile_avatar = string_field(map, "profileAvatar", &mut warnings);
        config.shortcuts = shortcut_settings_field(map, &mut warnings);
        config.layout = layout_field(map, &mut warnings);
        if let Some(raw) = map.get("memberships") {
            match serde_json::from_value::<Vec<Membership>>(raw.clone()) {
                Ok(memberships) => config.memberships = memberships,
                Err(error) => warnings.push(format!("memberships invalide : {error}")),
            }
        }
        config.normalize();

        for (key, value) in map {
            if !KNOWN_KEYS.contains(&key.as_str()) {
                config.extra.insert(key.clone(), value.clone());
            }
        }

        (config, warnings)
    }

    /// Applique les normalisations legacy (volume borné, dictionnaire nettoyé).
    pub fn normalize(&mut self) {
        self.volume = normalize_volume(self.volume);
        self.media_bindings = normalize_media_bindings(&self.media_bindings);
        for membership in &mut self.memberships {
            membership.volume = normalize_volume(membership.volume);
        }
        self.shortcuts.normalize();
        self.layout.normalize();
        self.reconcile_memberships();
    }

    /// Maintient la cohérence entre la liste des adhésions et les champs
    /// historiques (miroir de la guilde active).
    ///
    /// - une configuration d'avant le multi-guilde (champs historiques seuls)
    ///   devient une adhésion ;
    /// - les champs historiques font foi pour la guilde active (ce sont eux que
    ///   le code de session modifie) : ils sont recopiés dans son adhésion ;
    /// - si la guilde active a perdu son jeton (purge après révocation), son
    ///   adhésion est retirée et la première restante devient active.
    pub fn reconcile_memberships(&mut self) {
        self.memberships.retain(Membership::is_valid);
        let mut seen = Vec::new();
        self.memberships.retain(|membership| {
            if seen.contains(&membership.guild_id) {
                false
            } else {
                seen.push(membership.guild_id.clone());
                true
            }
        });

        if self.has_pairing_config() {
            let guild_id = self.guild_id.clone().unwrap_or_default();
            let mirrored = Membership {
                server_url: self.server_url.clone().unwrap_or_default(),
                guild_id: guild_id.clone(),
                client_token: self.client_token.clone().unwrap_or_default(),
                client_id: self.client_id.clone(),
                guest_mode: self.guest_mode,
                media_bindings: self.media_bindings.clone(),
                ..Membership::default()
            };
            match self.membership_mut(&guild_id) {
                Some(existing) => {
                    existing.server_url = mirrored.server_url;
                    existing.client_token = mirrored.client_token;
                    existing.client_id = mirrored.client_id;
                    existing.guest_mode = mirrored.guest_mode;
                    existing.media_bindings = mirrored.media_bindings;
                }
                None => self.memberships.push(Membership {
                    display_name: self.author_name.clone(),
                    ..mirrored
                }),
            }
            return;
        }

        // Guilde active sans jeton : elle a été purgée.
        if let Some(purged) = self.guild_id.clone() {
            self.memberships
                .retain(|membership| membership.guild_id != purged);
        }
        match self.memberships.first().cloned() {
            Some(next) => self.mirror(&next),
            None => {
                self.client_token = None;
                self.client_id = None;
                self.guild_id = None;
                self.guest_mode = false;
            }
        }
    }

    /// Recopie une adhésion dans les champs historiques (elle devient active).
    fn mirror(&mut self, membership: &Membership) {
        self.server_url = Some(membership.server_url.clone());
        self.client_token = Some(membership.client_token.clone());
        self.client_id = membership.client_id.clone();
        self.guild_id = Some(membership.guild_id.clone());
        self.guest_mode = membership.guest_mode;
        self.media_bindings = membership.media_bindings.clone();
    }

    /// Adhésion d'une guilde.
    pub fn membership(&self, guild_id: &str) -> Option<&Membership> {
        self.memberships
            .iter()
            .find(|membership| membership.guild_id == guild_id)
    }

    fn membership_mut(&mut self, guild_id: &str) -> Option<&mut Membership> {
        self.memberships
            .iter_mut()
            .find(|membership| membership.guild_id == guild_id)
    }

    /// `true` si `guild_id` est la guilde active.
    pub fn is_active_guild(&self, guild_id: &str) -> bool {
        self.guild_id.as_deref() == Some(guild_id)
    }

    /// Ajoute ou remplace une adhésion et la rend active.
    pub fn join(&mut self, membership: Membership) {
        // Fige d'abord l'état de la guilde active courante dans son adhésion.
        self.reconcile_memberships();
        self.memberships
            .retain(|existing| existing.guild_id != membership.guild_id);
        self.mirror(&membership);
        self.memberships.push(membership);
    }

    /// Rend une guilde active. `false` si elle est inconnue.
    pub fn activate(&mut self, guild_id: &str) -> bool {
        self.reconcile_memberships();
        match self.membership(guild_id).cloned() {
            Some(membership) => {
                self.mirror(&membership);
                true
            }
            None => false,
        }
    }

    /// Modifie une adhésion ; si elle est active, le miroir suit.
    /// `false` si la guilde est inconnue.
    pub fn update_membership<F>(&mut self, guild_id: &str, mutate: F) -> bool
    where
        F: FnOnce(&mut Membership),
    {
        self.reconcile_memberships();
        let Some(membership) = self.membership_mut(guild_id) else {
            return false;
        };
        mutate(membership);
        let updated = membership.clone();
        if self.is_active_guild(guild_id) {
            self.mirror(&updated);
        }
        true
    }

    /// Retire une adhésion. Si elle était active, la suivante le devient.
    pub fn leave(&mut self, guild_id: &str) -> bool {
        self.reconcile_memberships();
        let before = self.memberships.len();
        self.memberships
            .retain(|membership| membership.guild_id != guild_id);
        if self.memberships.len() == before {
            return false;
        }
        if self.is_active_guild(guild_id) {
            self.client_token = None;
            self.client_id = None;
            self.guild_id = None;
            self.guest_mode = false;
            self.media_bindings = MediaBindings::new();
            self.reconcile_memberships();
        }
        true
    }

    /// Vues des adhésions pour l'interface (sans jeton).
    pub fn membership_views(&self) -> Vec<MembershipView> {
        self.memberships
            .iter()
            .map(|membership| MembershipView {
                server_url: membership.server_url.clone(),
                guild_id: membership.guild_id.clone(),
                guild_name: membership.guild_name.clone(),
                guild_icon_url: membership.guild_icon_url.clone(),
                client_id: membership.client_id.clone(),
                member_id: membership.member_id.clone(),
                display_name: membership.display_name.clone(),
                role: membership.role.clone(),
                guest_mode: membership.guest_mode,
                muted: membership.muted,
                active: self.is_active_guild(&membership.guild_id),
            })
            .collect()
    }

    /// Projection renderer (8 champs).
    pub fn view(&self) -> OverlayConfigView {
        OverlayConfigView {
            server_url: self.server_url.clone(),
            client_token: self.client_token.clone(),
            guild_id: self.guild_id.clone(),
            client_id: self.client_id.clone(),
            guest_mode: self.guest_mode,
            show_text: self.show_text,
            volume: self.volume,
            media_bindings: self.media_bindings.clone(),
            layout: self.layout.clone(),
        }
    }

    /// `Boolean(serverUrl && clientToken && guildId)` du legacy.
    pub fn has_pairing_config(&self) -> bool {
        non_empty(&self.server_url) && non_empty(&self.client_token) && non_empty(&self.guild_id)
    }

    /// Sérialise en JSON pretty (2 espaces, comme `JSON.stringify(config, null, 2)`).
    pub fn to_json_string(&self) -> Result<String, ConfigError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Écrit le fichier de façon **atomique** : sérialisation dans un fichier
    /// temporaire voisin, `sync_all`, puis renommage sur la cible (NTFS remplace
    /// la cible de façon atomique). Crée le dossier parent si nécessaire.
    ///
    /// Un crash pendant l'écriture ne peut donc jamais laisser un `config.json`
    /// tronqué : soit l'ancien fichier reste, soit le nouveau est complet
    /// (AUD-A6-007 / O-006).
    pub fn write_to(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let payload = self.to_json_string()?;
        let temp = temp_sibling(path);
        {
            let mut file = fs::File::create(&temp)?;
            if let Err(error) = file
                .write_all(payload.as_bytes())
                .and_then(|()| file.sync_all())
            {
                let _removed = fs::remove_file(&temp);
                return Err(error.into());
            }
        }
        if let Err(error) = replace_file(&temp, path) {
            let _removed = fs::remove_file(&temp);
            return Err(error.into());
        }
        Ok(())
    }
}

/// Chemin temporaire voisin (même volume ⇒ renommage atomique).
fn temp_sibling(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    path.with_file_name(name)
}

/// Remplace `target` par `temp`. `std::fs::rename` mappe `MoveFileExW` avec
/// `MOVEFILE_REPLACE_EXISTING` sur Windows ; un antivirus/indexeur peut
/// brièvement retenir un handle, d'où quelques tentatives courtes.
fn replace_file(temp: &Path, target: &Path) -> io::Result<()> {
    let mut last_error = None;
    for attempt in 0..3 {
        match fs::rename(temp, target) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("renommage atomique impossible")))
}

fn non_empty(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|text| !text.trim().is_empty())
}

fn string_field(map: &Map<String, Value>, key: &str, warnings: &mut Vec<String>) -> Option<String> {
    match map.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(other) => {
            warnings.push(format!(
                "champ {key} ignoré (attendu: chaîne, reçu: {other})"
            ));
            None
        }
    }
}

fn int_field(map: &Map<String, Value>, key: &str, warnings: &mut Vec<String>) -> Option<i64> {
    match map.get(key) {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_i64() {
            Some(number) => Some(number),
            None => {
                warnings.push(format!(
                    "champ {key} ignoré (attendu: entier, reçu: {value})"
                ));
                None
            }
        },
    }
}

fn bool_field(
    map: &Map<String, Value>,
    key: &str,
    default: bool,
    warnings: &mut Vec<String>,
) -> bool {
    match map.get(key) {
        None => default,
        Some(Value::Bool(value)) => *value,
        Some(other) => {
            warnings.push(format!(
                "champ {key} ignoré (attendu: booléen, reçu: {other}); défaut {default} utilisé"
            ));
            default
        }
    }
}

fn volume_field(map: &Map<String, Value>, warnings: &mut Vec<String>) -> f64 {
    match map.get("volume") {
        None => DEFAULT_VOLUME,
        Some(value) => match value.as_f64() {
            Some(number) => normalize_volume(number),
            None => {
                warnings.push(format!(
                    "champ volume ignoré (attendu: nombre, reçu: {value})"
                ));
                DEFAULT_VOLUME
            }
        },
    }
}

fn media_bindings_field(map: &Map<String, Value>, warnings: &mut Vec<String>) -> MediaBindings {
    match map.get("mediaBindings") {
        None | Some(Value::Null) => MediaBindings::new(),
        Some(Value::Object(bindings)) => {
            let mut raw = MediaBindings::new();
            for (accelerator, item_id) in bindings {
                let accelerator = accelerator.trim().to_string();
                let item_id = match item_id {
                    Value::String(text) => text.trim().to_string(),
                    Value::Null => String::new(),
                    Value::Number(number) => number.to_string(),
                    Value::Bool(value) => value.to_string(),
                    other => {
                        warnings.push(format!(
                            "binding {accelerator} ignoré (identifiant d'item non scalaire: {other})"
                        ));
                        continue;
                    }
                };
                raw.insert(accelerator, item_id);
            }
            normalize_media_bindings(&raw)
        }
        Some(other) => {
            warnings.push(format!(
                "champ mediaBindings ignoré (attendu: objet, reçu: {other})"
            ));
            MediaBindings::new()
        }
    }
}

fn shortcut_settings_field(
    map: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> ShortcutSettings {
    let Some(raw) = map.get("shortcuts") else {
        return ShortcutSettings::default();
    };
    if raw.is_null() {
        return ShortcutSettings::default();
    }
    let Some(object) = raw.as_object() else {
        warnings.push(format!(
            "champ shortcuts ignoré (attendu: objet, reçu: {raw})"
        ));
        return ShortcutSettings::default();
    };
    ShortcutSettings {
        stop: string_field(object, "stop", warnings),
        mute_all: string_field(object, "muteAll", warnings),
    }
}

fn layout_field(map: &Map<String, Value>, warnings: &mut Vec<String>) -> OverlayLayout {
    let Some(raw) = map.get("layout") else {
        return OverlayLayout::default();
    };
    if raw.is_null() {
        return OverlayLayout::default();
    }
    let Some(object) = raw.as_object() else {
        warnings.push(format!("champ layout ignoré (attendu: objet, reçu: {raw})"));
        return OverlayLayout::default();
    };
    let mut layout = OverlayLayout::default();
    if let Some(align) = object.get("align") {
        match serde_json::from_value::<LayoutAnchor>(align.clone()) {
            Ok(parsed) => layout.align = parsed,
            Err(_) => warnings.push(format!(
                "layout.align ignoré (valeur inconnue: {align}); défaut center utilisé"
            )),
        }
    }
    match object.get("zone") {
        None | Some(Value::Null) => {}
        Some(Value::Object(zone)) => {
            for (key, slot) in [
                ("x", &mut layout.zone.x),
                ("y", &mut layout.zone.y),
                ("width", &mut layout.zone.width),
                ("height", &mut layout.zone.height),
            ] {
                if let Some(value) = zone_number_field(zone, key, warnings) {
                    *slot = value;
                }
            }
        }
        Some(raw_zone) => warnings.push(format!(
            "layout.zone ignoré (attendu: objet, reçu: {raw_zone})"
        )),
    }
    layout.normalize();
    layout
}

fn zone_number_field(
    zone: &Map<String, Value>,
    key: &str,
    warnings: &mut Vec<String>,
) -> Option<f64> {
    match zone.get(key) {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_f64() {
            Some(number) if number.is_finite() => Some(number),
            _ => {
                warnings.push(format!(
                    "layout.zone.{key} ignoré (attendu: nombre, reçu: {value})"
                ));
                None
            }
        },
    }
}

/// `Math.min(1, Math.max(0, volume))` legacy, défaut 1 pour les valeurs non finies.
pub fn normalize_volume(volume: f64) -> f64 {
    if !volume.is_finite() {
        return DEFAULT_VOLUME;
    }
    volume.clamp(0.0, 1.0)
}

/// Trim des deux côtés et suppression des entrées vides (legacy).
pub fn normalize_media_bindings(bindings: &MediaBindings) -> MediaBindings {
    let mut normalized = MediaBindings::new();
    for (accelerator, item_id) in bindings {
        let accelerator = accelerator.trim();
        let item_id = item_id.trim();
        if !accelerator.is_empty() && !item_id.is_empty() {
            normalized.insert(accelerator.to_string(), item_id.to_string());
        }
    }
    normalized
}

/// `normalizeServerUrl` legacy : trim + suppression des `/` finaux.
pub fn normalize_server_url(raw: Option<&str>) -> Option<String> {
    let trimmed = raw.unwrap_or_default().trim().trim_end_matches('/');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Lecture tolérante du fichier : fichier absent, JSON invalide ou champs
/// invalides retombent sur les défauts (avec un warning) comme le legacy.
pub fn load_from(path: &Path) -> OverlayConfig {
    match fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(value) => {
                let (config, warnings) = OverlayConfig::from_json_value(&value);
                for warning in warnings {
                    tracing::warn!(target: "lc_overlay::config", chemin = %path.display(), "{warning}");
                }
                config
            }
            Err(error) => {
                tracing::warn!(target: "lc_overlay::config", chemin = %path.display(), %error, "config.json illisible, défauts utilisés");
                OverlayConfig::default()
            }
        },
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(target: "lc_overlay::config", chemin = %path.display(), %error, "lecture config impossible, défauts utilisés");
            }
            OverlayConfig::default()
        }
    }
}

/// Erreurs de persistance de la configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("accès au fichier de configuration impossible: {0}")]
    Io(#[from] io::Error),
    #[error("sérialisation de la configuration impossible: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Accès disque au fichier de configuration (chemin forcé, .bak, migration).
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn backup_path(&self) -> PathBuf {
        let mut backup = self.path.clone().into_os_string();
        backup.push(CONFIG_BACKUP_SUFFIX);
        PathBuf::from(backup)
    }

    pub fn load(&self) -> OverlayConfig {
        load_from(&self.path)
    }

    pub fn save(&self, config: &OverlayConfig) -> Result<(), ConfigError> {
        config.write_to(&self.path)
    }

    /// Crée `.bak` une seule fois (avant toute écriture Tauri) pour préserver
    /// le fichier lu par Electron en cas de rollback. La copie passe par un
    /// temporaire voisin renommé ensuite : un crash ne laisse pas de `.bak`
    /// partiel (AUD-A6-007).
    pub fn ensure_backup(&self) -> Result<bool, ConfigError> {
        if !self.path.exists() || self.backup_path().exists() {
            return Ok(false);
        }
        let backup = self.backup_path();
        let temp = temp_sibling(&backup);
        if let Err(error) = fs::copy(&self.path, &temp) {
            let _removed = fs::remove_file(&temp);
            return Err(error.into());
        }
        if let Err(error) = replace_file(&temp, &backup) {
            let _removed = fs::remove_file(&temp);
            return Err(error.into());
        }
        Ok(true)
    }

    /// Migration douce : si le fichier forcé n'existe pas et qu'un fichier
    /// `candidate` existe (ancien `app_config_dir()/config.json`), le copie.
    pub fn migrate_from(&self, candidate: &Path) -> Result<bool, ConfigError> {
        if self.path.exists() || candidate == self.path || !candidate.exists() {
            return Ok(false);
        }
        fs::copy(candidate, &self.path)?;
        Ok(true)
    }
}

/// Résout le chemin de configuration : override env, puis `%APPDATA%\LiveChat`,
/// puis fallback `app_config_dir()` (jamais utilisé en pratique, sécurité).
pub fn resolve_config_path(
    appdata: Option<&Path>,
    fallback_app_config_dir: Option<&Path>,
) -> PathBuf {
    if let Some(raw) = std::env::var_os(CONFIG_PATH_ENV) {
        if !raw.is_empty() {
            return PathBuf::from(raw);
        }
    }
    if let Some(appdata) = appdata {
        return appdata.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME);
    }
    if let Some(fallback) = fallback_app_config_dir {
        return fallback.join(CONFIG_FILE_NAME);
    }
    PathBuf::from(CONFIG_FILE_NAME)
}

/// `%APPDATA%` de l'utilisateur courant, si défini.
pub fn appdata_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Résolution standard au démarrage (override env > `%APPDATA%\LiveChat`).
pub fn resolve_config_path_from_env(fallback_app_config_dir: Option<&Path>) -> PathBuf {
    resolve_config_path(appdata_dir().as_deref(), fallback_app_config_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn temp_store(config: &str) -> (tempfile::TempDir, ConfigStore) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("LiveChat").join("config.json");
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, config).expect("write initial");
        (dir, ConfigStore::new(path))
    }

    #[test]
    fn default_config_matches_legacy() {
        let config = OverlayConfig::default();
        assert_eq!(config.volume, 1.0);
        assert!(config.enabled);
        assert!(!config.auto_start);
        assert!(!config.guest_mode);
        assert!(config.show_text);
        assert_eq!(config.display_id, None);
        assert_eq!(config.display_index, None);
        assert_eq!(config.display_key, None);
        assert_eq!(config.display_label, None);
        assert_eq!(config.server_url, None);
        assert_eq!(config.client_token, None);
        assert_eq!(config.client_id, None);
        assert_eq!(config.guild_id, None);
        assert_eq!(config.author_name, None);
        assert_eq!(config.device_name, None);
        assert!(config.media_bindings.is_empty());
        assert_eq!(config.shortcuts, ShortcutSettings::default());
        assert_eq!(config.layout, OverlayLayout::default());
        assert!(config.extra.is_empty());
    }

    #[test]
    fn partial_json_merges_with_defaults() {
        let (config, warnings) =
            OverlayConfig::from_json_value(&json!({"volume": 0.5, "enabled": false}));
        assert_eq!(config.volume, 0.5);
        assert!(!config.enabled);
        assert!(config.show_text);
        assert!(warnings.is_empty());
    }

    #[test]
    fn shortcuts_and_layout_are_parsed_and_normalized() {
        let (config, warnings) = OverlayConfig::from_json_value(&json!({
            "shortcuts": {
                "stop": "  Ctrl+Shift+S  ",
                "muteAll": "   "
            },
            "layout": {
                "align": "top-left",
                "zone": { "x": 0.9, "y": -0.2, "width": 0.25, "height": 0.01 }
            }
        }));
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(config.shortcuts.stop.as_deref(), Some("Ctrl+Shift+S"));
        assert!(config.shortcuts.mute_all.is_none());
        assert_eq!(config.layout.align, LayoutAnchor::TopLeft);
        assert_eq!(
            config.layout.zone,
            LayoutZone {
                x: 0.75,
                y: 0.0,
                width: 0.25,
                height: MIN_ZONE_FRACTION,
            },
            "the zone is kept on screen and not smaller than the minimum"
        );
    }

    #[test]
    fn invalid_layout_values_fall_back_to_the_defaults() {
        let (config, warnings) = OverlayConfig::from_json_value(&json!({
            "layout": { "align": "middle", "zone": { "x": "left", "width": 0.5 } }
        }));
        assert_eq!(config.layout.align, LayoutAnchor::Center);
        assert_eq!(config.layout.zone.x, 0.0);
        assert_eq!(config.layout.zone.width, 0.5);
        for field in ["layout.align", "layout.zone.x"] {
            assert!(
                warnings.iter().any(|warning| warning.contains(field)),
                "{field} missing from {warnings:?}"
            );
        }
    }

    #[test]
    fn the_former_layout_keys_are_dropped() {
        let (config, _warnings) = OverlayConfig::from_json_value(&json!({
            "layout": { "anchor": "top-left", "maxWidth": 50, "margin": 10 }
        }));
        assert_eq!(config.layout, OverlayLayout::default());
    }

    #[test]
    fn a_non_finite_zone_resets_to_the_whole_screen() {
        let mut zone = LayoutZone {
            x: 0.2,
            y: f64::NAN,
            width: 0.5,
            height: 0.5,
        };
        zone.normalize();
        assert_eq!(zone, LayoutZone::default());
    }

    #[test]
    fn new_keys_roundtrip_through_the_config_file() {
        let (_dir, store) = temp_store("{}");
        let mut config = store.load();
        config.shortcuts.stop = Some("Ctrl+Shift+S".to_string());
        config.shortcuts.mute_all = Some("Ctrl+Alt+M".to_string());
        config.layout = OverlayLayout {
            zone: LayoutZone {
                x: 0.5,
                y: 0.25,
                width: 0.4,
                height: 0.3,
            },
            align: LayoutAnchor::BottomRight,
        };
        config.join(Membership {
            server_url: "https://example.test".to_string(),
            guild_id: "111".to_string(),
            client_token: "jeton".to_string(),
            volume: 0.5,
            ..Membership::default()
        });
        store.save(&config).expect("save");

        let raw = fs::read_to_string(store.path()).expect("read saved config");
        for key in ["shortcuts", "layout", "\"volume\": 0.5"] {
            assert!(raw.contains(key), "missing {key} in {raw}");
        }
        let reloaded = store.load();
        assert_eq!(reloaded.shortcuts, config.shortcuts);
        assert_eq!(reloaded.layout, config.layout);
        assert_eq!(
            reloaded
                .membership("111")
                .map(|membership| membership.volume),
            Some(0.5)
        );
    }

    #[test]
    fn membership_volume_defaults_to_one_and_is_clamped() {
        assert_eq!(Membership::default().volume, 1.0);
        let (config, _) = OverlayConfig::from_json_value(&json!({
            "memberships": [{
                "serverUrl": "https://example.test",
                "guildId": "111",
                "clientToken": "jeton",
                "volume": 3.0
            }]
        }));
        assert_eq!(
            config.membership("111").map(|membership| membership.volume),
            Some(1.0)
        );
    }

    #[test]
    fn invalid_fields_fall_back_to_defaults_with_warnings() {
        let (config, warnings) = OverlayConfig::from_json_value(&json!({
            "volume": "fort",
            "enabled": 3,
            "autoStart": "true",
            "guestMode": 1,
            "showText": null,
            "displayId": "ecran",
            "serverUrl": 12
        }));
        assert_eq!(config.volume, 1.0);
        assert!(config.enabled);
        assert!(!config.auto_start);
        assert!(!config.guest_mode);
        assert!(config.show_text);
        assert_eq!(config.display_id, None);
        assert_eq!(config.server_url, None);
        assert!(warnings.len() >= 5, "warnings: {warnings:?}");
    }

    #[test]
    fn volume_is_clamped_and_non_finite_uses_default() {
        assert_eq!(normalize_volume(-2.0), 0.0);
        assert_eq!(normalize_volume(3.5), 1.0);
        assert_eq!(normalize_volume(0.25), 0.25);
        assert_eq!(normalize_volume(f64::NAN), 1.0);
        assert_eq!(normalize_volume(f64::INFINITY), 1.0);
    }

    #[test]
    fn media_bindings_are_trimmed_and_empties_dropped() {
        let (config, _) = OverlayConfig::from_json_value(&json!({
            "mediaBindings": {
                "  Ctrl+Shift+1  ": "  media-1  ",
                "   ": "media-2",
                "Ctrl+Alt+2": "   ",
                "Ctrl+Alt+3": 7
            }
        }));
        assert_eq!(
            config.media_bindings,
            BTreeMap::from([
                ("Ctrl+Alt+3".to_string(), "7".to_string()),
                ("Ctrl+Shift+1".to_string(), "media-1".to_string()),
            ])
        );
    }

    #[test]
    fn guild_icon_url_roundtrips_through_the_config_file() {
        let (_dir, store) = temp_store("{}");
        let mut config = store.load();
        config.join(Membership {
            server_url: "https://example.test".to_string(),
            guild_id: "111".to_string(),
            guild_name: Some("Les Sardines".to_string()),
            guild_icon_url: Some("https://cdn.discordapp.com/icons/111/abc.png".to_string()),
            client_token: "jeton".to_string(),
            ..Membership::default()
        });
        store.save(&config).expect("save");

        let raw = fs::read_to_string(store.path()).expect("read saved config");
        assert!(raw.contains("\"guildIconUrl\""), "json: {raw}");

        let reloaded = store.load();
        let membership = reloaded.membership("111").expect("adhésion");
        assert_eq!(
            membership.guild_icon_url.as_deref(),
            Some("https://cdn.discordapp.com/icons/111/abc.png")
        );
        let views = reloaded.membership_views();
        assert_eq!(
            views[0].guild_icon_url.as_deref(),
            Some("https://cdn.discordapp.com/icons/111/abc.png")
        );
    }

    #[test]
    fn membership_without_guild_icon_url_loads_with_none() {
        let (config, warnings) = OverlayConfig::from_json_value(&json!({
            "memberships": [{
                "serverUrl": "https://example.test",
                "guildId": "111",
                "guildName": "Les Sardines",
                "clientToken": "jeton"
            }]
        }));
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let membership = config.membership("111").expect("adhésion");
        assert_eq!(membership.guild_name.as_deref(), Some("Les Sardines"));
        assert!(membership.guild_icon_url.is_none());
    }

    #[test]
    fn roundtrip_preserves_known_fields_and_extras() {
        let (_dir, store) = temp_store("{}");
        let mut config = store.load();
        config.volume = 0.4;
        config.auto_start = true;
        config.display_index = Some(1);
        config.display_key = Some("0,0,2560,1440,0".to_string());
        config.display_label = Some("Écran 1".to_string());
        config.server_url = Some("http://localhost:3000/".to_string());
        config.media_bindings = BTreeMap::from([("Ctrl+1".to_string(), "id-1".to_string())]);
        config
            .extra
            .insert("electronOnly".to_string(), json!({"nested": true}));
        store.save(&config).expect("save");

        let reloaded = store.load();
        assert_eq!(reloaded, config);
        assert_eq!(
            reloaded.server_url.as_deref(),
            Some("http://localhost:3000/")
        );
        assert_eq!(
            reloaded.extra.get("electronOnly"),
            Some(&json!({"nested": true}))
        );
    }

    #[test]
    fn pretty_json_output_has_two_space_indent() {
        let config = OverlayConfig::default();
        let text = config.to_json_string().expect("serialize");
        assert!(text.contains("\n  \"volume\": 1.0"), "json: {text}");
        assert!(!text.ends_with('\n'));
    }

    #[test]
    fn corrupted_file_falls_back_to_defaults() {
        let (_dir, store) = temp_store("{ ceci n'est pas du json");
        assert_eq!(store.load(), OverlayConfig::default());
    }

    #[test]
    fn non_object_json_falls_back_to_defaults() {
        let (_dir, store) = temp_store("[]");
        assert_eq!(store.load(), OverlayConfig::default());
    }

    #[test]
    fn backup_is_created_only_once() {
        let (_dir, store) = temp_store("{\"volume\":0.2}");
        assert!(store.ensure_backup().expect("backup"));
        let backup = fs::read_to_string(store.backup_path()).expect("read backup");
        assert!(backup.contains("0.2"));
        assert!(!store.ensure_backup().expect("second backup"));
    }

    #[test]
    fn backup_leaves_no_temporary_file() {
        let (dir, store) = temp_store("{\"volume\":0.2}");
        assert!(store.ensure_backup().expect("backup"));
        let leftovers = fs::read_dir(dir.path())
            .expect("readdir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(leftovers, 0, "aucun temporaire ne doit rester");
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temporary_file() {
        let (dir, store) = temp_store("{\"volume\":0.2}");
        let mut config = store.load();
        config.volume = 0.5;
        store.save(&config).expect("save");

        assert_eq!(store.load().volume, 0.5);
        let leftovers = fs::read_dir(dir.path())
            .expect("readdir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(leftovers, 0, "aucun temporaire ne doit rester");
        // La cible reste un JSON complet et valide après le renommage.
        let raw = fs::read_to_string(store.path()).expect("read target");
        assert!(raw.contains("\"volume\": 0.5"), "json: {raw}");
    }

    #[test]
    fn repeated_saves_replace_the_previous_file() {
        let (_dir, store) = temp_store("{\"volume\":0.2}");
        for volume in [0.1, 0.4, 0.9] {
            let mut config = store.load();
            config.volume = volume;
            store.save(&config).expect("save");
        }
        assert!((store.load().volume - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn migration_copies_candidate_only_when_target_missing() {
        let dir = tempdir().expect("tempdir");
        let candidate = dir.path().join("com.livechat").join("config.json");
        fs::create_dir_all(candidate.parent().expect("parent")).expect("mkdir");
        fs::write(&candidate, "{\"volume\":0.7}").expect("write candidate");
        let target = dir.path().join("LiveChat").join("config.json");
        fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        let store = ConfigStore::new(target.clone());

        assert!(store.migrate_from(&candidate).expect("migrate"));
        assert_eq!(store.load().volume, 0.7);
        assert!(!store.migrate_from(&candidate).expect("already migrated"));

        let store_with_target = ConfigStore::new(target);
        assert!(!store_with_target
            .migrate_from(&candidate)
            .expect("target exists"));
    }

    #[test]
    fn server_url_normalization_matches_legacy() {
        assert_eq!(
            normalize_server_url(Some("  http://h:3000// ")),
            Some("http://h:3000".to_string())
        );
        assert_eq!(normalize_server_url(Some("   ")), None);
        assert_eq!(normalize_server_url(None), None);
        assert_eq!(
            normalize_server_url(Some("https://livechat.example/")),
            Some("https://livechat.example".to_string())
        );
    }

    #[test]
    fn view_projects_renderer_fields_and_layout() {
        let mut config = OverlayConfig::default();
        config.server_url = Some("http://localhost:3000".to_string());
        config.client_token = Some("jeton".to_string());
        config.guild_id = Some("guilde".to_string());
        config.media_bindings = BTreeMap::from([("Ctrl+1".to_string(), "id-1".to_string())]);
        config.layout.align = LayoutAnchor::BottomRight;
        config.layout.zone.width = 0.5;
        let value = serde_json::to_value(config.view()).expect("serialize view");
        let object = value.as_object().expect("objet");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "clientId",
                "clientToken",
                "guestMode",
                "guildId",
                "layout",
                "mediaBindings",
                "serverUrl",
                "showText",
                "volume"
            ]
        );
        assert_eq!(object["serverUrl"], json!("http://localhost:3000"));
        assert_eq!(
            object["layout"],
            json!({
                "zone": { "x": 0.0, "y": 0.0, "width": 0.5, "height": 1.0 },
                "align": "bottom-right"
            })
        );
    }

    #[test]
    fn pairing_config_requires_three_non_empty_fields() {
        let mut config = OverlayConfig::default();
        assert!(!config.has_pairing_config());
        config.server_url = Some("http://localhost:3000".to_string());
        config.client_token = Some("jeton".to_string());
        assert!(!config.has_pairing_config());
        config.guild_id = Some("guilde".to_string());
        assert!(config.has_pairing_config());
        config.guild_id = Some("   ".to_string());
        assert!(!config.has_pairing_config());
    }

    #[test]
    fn resolve_config_path_prefers_appdata_then_fallback() {
        let appdata = Path::new(r"C:\Users\test\AppData\Roaming");
        let fallback = Path::new(r"C:\Users\test\AppData\Roaming\com.livechat");
        assert_eq!(
            resolve_config_path(Some(appdata), Some(fallback)),
            appdata.join("LiveChat").join("config.json")
        );
        assert_eq!(
            resolve_config_path(None, Some(fallback)),
            fallback.join("config.json")
        );
    }
}
