use std::path::Path;

use crate::atomic;

pub use typeclaw_host_config::{Config, ConfigSource, home_default};

/// Default config emitted by `typeclaw config init`. The TOML serializer drops comments,
/// so we keep the documented template as a string here.
pub const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../config/typeclaw.toml.template");

pub fn write_default_template(path: &Path) -> Result<(), String> {
    atomic::write(path, DEFAULT_CONFIG_TEMPLATE.as_bytes())
}

pub fn write_config(path: &Path, config: &Config) -> Result<(), String> {
    let serialized =
        toml::to_string_pretty(config).map_err(|e| format!("serialize config: {e}"))?;
    atomic::write(path, serialized.as_bytes())
}
