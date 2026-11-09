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
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
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

const KNOWN_KEYS: [&str; 16] = [
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
    "memeBindings",
];

/// Dictionnaire accélérateur -> identifiant de média (meme board).
pub type MemeBindings = BTreeMap<String, String>;

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
    pub meme_bindings: MemeBindings,
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
            meme_bindings: MemeBindings::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// Projection envoyée au renderer (8 champs, cf. `OverlayConfig` legacy).
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
    pub meme_bindings: MemeBindings,
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
        config.meme_bindings = meme_bindings_field(map, &mut warnings);

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
        self.meme_bindings = normalize_meme_bindings(&self.meme_bindings);
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
            meme_bindings: self.meme_bindings.clone(),
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

    /// Écrit le fichier (crée le dossier parent si nécessaire).
    pub fn write_to(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, self.to_json_string()?)?;
        Ok(())
    }
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

fn meme_bindings_field(map: &Map<String, Value>, warnings: &mut Vec<String>) -> MemeBindings {
    match map.get("memeBindings") {
        None | Some(Value::Null) => MemeBindings::new(),
        Some(Value::Object(bindings)) => {
            let mut raw = MemeBindings::new();
            for (accelerator, item_id) in bindings {
                let accelerator = accelerator.trim().to_string();
                let item_id = match item_id {
                    Value::String(text) => text.trim().to_string(),
                    Value::Null => String::new(),
                    Value::Number(number) => number.to_string(),
                    Value::Bool(value) => value.to_string(),
                    other => {
                        warnings.push(format!(
                            "binding {accelerator} ignoré (identifiant de média non scalaire: {other})"
                        ));
                        continue;
                    }
                };
                raw.insert(accelerator, item_id);
            }
            normalize_meme_bindings(&raw)
        }
        Some(other) => {
            warnings.push(format!(
                "champ memeBindings ignoré (attendu: objet, reçu: {other})"
            ));
            MemeBindings::new()
        }
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
pub fn normalize_meme_bindings(bindings: &MemeBindings) -> MemeBindings {
    let mut normalized = MemeBindings::new();
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
    /// le fichier lu par Electron en cas de rollback.
    pub fn ensure_backup(&self) -> Result<bool, ConfigError> {
        if !self.path.exists() || self.backup_path().exists() {
            return Ok(false);
        }
        fs::copy(&self.path, self.backup_path())?;
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
        assert!(config.meme_bindings.is_empty());
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
    fn meme_bindings_are_trimmed_and_empties_dropped() {
        let (config, _) = OverlayConfig::from_json_value(&json!({
            "memeBindings": {
                "  Ctrl+Shift+1  ": "  media-1  ",
                "   ": "media-2",
                "Ctrl+Alt+2": "   ",
                "Ctrl+Alt+3": 7
            }
        }));
        assert_eq!(
            config.meme_bindings,
            BTreeMap::from([
                ("Ctrl+Alt+3".to_string(), "7".to_string()),
                ("Ctrl+Shift+1".to_string(), "media-1".to_string()),
            ])
        );
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
        config.meme_bindings = BTreeMap::from([("Ctrl+1".to_string(), "id-1".to_string())]);
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
    fn view_projects_eight_fields_only() {
        let mut config = OverlayConfig::default();
        config.server_url = Some("http://localhost:3000".to_string());
        config.client_token = Some("jeton".to_string());
        config.guild_id = Some("guilde".to_string());
        config.meme_bindings = BTreeMap::from([("Ctrl+1".to_string(), "id-1".to_string())]);
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
                "memeBindings",
                "serverUrl",
                "showText",
                "volume"
            ]
        );
        assert_eq!(object["serverUrl"], json!("http://localhost:3000"));
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
