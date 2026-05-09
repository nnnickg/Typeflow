use std::fs;
use std::path::Path;

pub use typeflow_core::host_config::{Config, ConfigSource, home_default};

/// Default config emitted by `typeflow config init`. The TOML serializer drops comments,
/// so we keep the documented template as a string here.
pub const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../config/typeflow.toml.template");

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
