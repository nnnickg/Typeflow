use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use typeflow_core::EngineConfig;

/// Default config emitted by `typeflow config init`. The TOML serializer drops comments,
/// so we keep the documented template as a string here.
pub const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../config/typeflow.toml.template");

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub engine: EngineSection,
    pub language: LanguageSection,
    pub packs: PacksSection,
    pub apps: AppsSection,
    pub hotkey: HotkeySection,
    pub data: DataSection,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct EngineSection {
    pub min_token_len: usize,
    pub confidence_margin: f32,
    pub dict_exact_weight: f32,
    pub dict_prefix_weight: f32,
    pub ngram_only_confidence_margin: f32,
    pub bigram_weight: f32,
    pub trigram_weight: f32,
    pub length_normalize: bool,
    pub disable_on_internal_caps: bool,
}

impl Default for EngineSection {
    fn default() -> Self {
        let defaults = EngineConfig::default();
        Self {
            min_token_len: defaults.min_token_len,
            confidence_margin: defaults.confidence_margin,
            dict_exact_weight: defaults.dict_exact_weight,
            dict_prefix_weight: defaults.dict_prefix_weight,
            ngram_only_confidence_margin: defaults.ngram_only_confidence_margin,
            bigram_weight: defaults.bigram_weight,
            trigram_weight: defaults.trigram_weight,
            length_normalize: defaults.length_normalize,
            disable_on_internal_caps: defaults.disable_on_internal_caps,
        }
    }
}

impl From<EngineSection> for EngineConfig {
    fn from(value: EngineSection) -> Self {
        Self {
            min_token_len: value.min_token_len,
            confidence_margin: value.confidence_margin,
            dict_exact_weight: value.dict_exact_weight,
            dict_prefix_weight: value.dict_prefix_weight,
            ngram_only_confidence_margin: value.ngram_only_confidence_margin,
            bigram_weight: value.bigram_weight,
            trigram_weight: value.trigram_weight,
            length_normalize: value.length_normalize,
            disable_on_internal_caps: value.disable_on_internal_caps,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct LanguageSection {
    pub secondary: String,
}

impl Default for LanguageSection {
    fn default() -> Self {
        Self {
            secondary: "uk".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PacksSection {
    pub directory: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AppsSection {
    pub exclude_bundle_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct HotkeySection {
    pub manual_convert: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DataSection {
    pub directory: Option<PathBuf>,
}

/// Tracks which file (if any) provided the loaded config.
#[derive(Debug, Default)]
pub struct ConfigSource {
    pub config: Config,
    pub path: Option<PathBuf>,
}

impl Config {
    /// Resolves the config in precedence order:
    /// 1. `--config <path>` if supplied,
    /// 2. `$TYPEFLOW_CONFIG` if set,
    /// 3. `~/.config/typeflow/config.toml` if it exists,
    /// 4. compile-time defaults.
    pub fn load(explicit: Option<&Path>) -> Result<ConfigSource, String> {
        if let Some(path) = explicit {
            return read_path(path).map(|config| ConfigSource {
                config,
                path: Some(path.to_path_buf()),
            });
        }
        if let Ok(env_path) = env::var("TYPEFLOW_CONFIG") {
            let path = PathBuf::from(env_path);
            return read_path(&path).map(|config| ConfigSource {
                config,
                path: Some(path),
            });
        }
        if let Some(home_path) = home_default()
            && home_path.is_file()
        {
            return read_path(&home_path).map(|config| ConfigSource {
                config,
                path: Some(home_path),
            });
        }
        Ok(ConfigSource {
            config: Config::default(),
            path: None,
        })
    }
}

fn read_path(path: &Path) -> Result<Config, String> {
    let bytes =
        fs::read_to_string(path).map_err(|e| format!("read config {}: {e}", path.display()))?;
    toml::from_str::<Config>(&bytes).map_err(|e| format!("parse config {}: {e}", path.display()))
}

pub fn home_default() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/typeflow/config.toml"))
}

pub fn default_pack_dir() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;

    #[cfg(target_os = "macos")]
    {
        Some(
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Typeflow")
                .join("packs"),
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        Some(PathBuf::from(home).join(".local/share/typeflow/packs"))
    }
}

pub fn write_default_template(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    fs::write(path, DEFAULT_CONFIG_TEMPLATE).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn write_config(path: &Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let serialized =
        toml::to_string_pretty(config).map_err(|e| format!("serialize config: {e}"))?;
    fs::write(path, serialized).map_err(|e| format!("write {}: {e}", path.display()))
}
