use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::EngineConfig;
use crate::data::LanguageBundle;

const TERMINAL_BUNDLE_IDS: &[&str] = &[
    "co.zeit.hyper",
    "com.apple.Terminal",
    "com.github.wez.wezterm",
    "com.googlecode.iterm2",
    "com.mitchellh.ghostty",
    "dev.warp.Warp",
    "dev.warp.Warp-Preview",
    "dev.warp.Warp-Stable",
    "io.alacritty",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "org.wezfurlong.wezterm",
];

const TERMINAL_SURFACE_MARKERS: &[&str] = &[
    "alacritty",
    "console",
    "ghostty",
    "iterm",
    "kitty",
    "pty",
    "pseudo terminal",
    "shell",
    "terminal",
    "tty",
    "vt100",
    "warp",
    "wezterm",
    "xterm",
];

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub engine: EngineSection,
    pub language: LanguageSection,
    pub packs: PacksSection,
    pub apps: AppsSection,
    pub data: DataSection,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct EngineSection {
    pub min_token_len: usize,
    pub max_token_len: usize,
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
            max_token_len: defaults.max_token_len,
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
            max_token_len: value.max_token_len,
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
    pub disable_bundle_ids: Vec<String>,
    #[serde(alias = "exclude_bundle_ids")]
    pub disable_auto_bundle_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DataSection {
    pub directory: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct ConfigSource {
    pub config: Config,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct ResolvedHostConfig {
    pub engine: EngineConfig,
    pub secondary_language: String,
    pub pack_directory: Option<PathBuf>,
    pub data_directory: Option<PathBuf>,
    pub app_policy: AppDisablePolicy,
    pub source_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct HostEnvironment {
    pub config_path: Option<PathBuf>,
    pub data_directory: Option<PathBuf>,
    pub pack_directory: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct AppDisablePolicy {
    disable_bundle_ids: HashSet<String>,
    disable_auto_bundle_ids: HashSet<String>,
}

#[derive(Clone, Debug, Default)]
pub struct HostSurfaceFacts {
    pub secure_input: bool,
    pub bundle_id: Option<String>,
    pub application_name: Option<String>,
    pub input_client_class: Option<String>,
    pub focused_element_role: Option<String>,
    pub focused_element_subrole: Option<String>,
    pub focused_element_role_description: Option<String>,
    pub focused_element_identifier: Option<String>,
    pub focused_element_description: Option<String>,
    pub focused_window_title: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HostSurfaceFactsView<'a> {
    pub secure_input: bool,
    pub bundle_id: Option<&'a str>,
    pub application_name: Option<&'a str>,
    pub input_client_class: Option<&'a str>,
    pub focused_element_role: Option<&'a str>,
    pub focused_element_subrole: Option<&'a str>,
    pub focused_element_role_description: Option<&'a str>,
    pub focused_element_identifier: Option<&'a str>,
    pub focused_element_description: Option<&'a str>,
    pub focused_window_title: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostInputPolicyReason {
    Normal,
    SecureInput,
    TerminalBundle,
    TerminalSurface,
    DisabledBundle,
    AutomaticProcessingDisabledBundle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostInputPolicy {
    pub disable_automatic_processing: bool,
    pub disable_manual_conversion: bool,
    pub terminal_surface: bool,
    pub reason: HostInputPolicyReason,
}

impl Default for HostInputPolicy {
    fn default() -> Self {
        Self {
            disable_automatic_processing: false,
            disable_manual_conversion: false,
            terminal_surface: false,
            reason: HostInputPolicyReason::Normal,
        }
    }
}

impl Config {
    /// Resolves the config in precedence order:
    /// 1. explicit path if supplied,
    /// 2. `$TYPEFLOW_CONFIG` if set,
    /// 3. `~/.config/typeflow/config.toml` if it exists,
    /// 4. compile-time defaults.
    pub fn load(explicit: Option<&Path>) -> Result<ConfigSource, String> {
        Self::load_with_environment(explicit, &HostEnvironment::from_process())
    }

    pub fn load_with_environment(
        explicit: Option<&Path>,
        environment: &HostEnvironment,
    ) -> Result<ConfigSource, String> {
        if let Some(path) = explicit {
            return read_path(path).map(|config| ConfigSource {
                config,
                path: Some(path.to_path_buf()),
            });
        }
        if let Some(path) = environment.config_path.as_deref() {
            return read_path(path).map(|config| ConfigSource {
                config,
                path: Some(path.to_path_buf()),
            });
        }
        if let Some(home_path) = home_default_with_environment(environment)
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

impl ResolvedHostConfig {
    pub fn load(explicit: Option<&Path>) -> Result<Self, String> {
        Self::load_with_environment(explicit, &HostEnvironment::from_process())
    }

    pub fn load_with_environment(
        explicit: Option<&Path>,
        environment: &HostEnvironment,
    ) -> Result<Self, String> {
        let source = Config::load_with_environment(explicit, environment)?;
        Self::from_source(source, environment)
    }

    pub fn from_source(
        source: ConfigSource,
        environment: &HostEnvironment,
    ) -> Result<Self, String> {
        let engine: EngineConfig = source.config.engine.into();
        engine
            .validate()
            .map_err(|e| format!("invalid engine config: {e}"))?;

        Ok(Self {
            engine,
            secondary_language: normalized_secondary_id(&source.config),
            pack_directory: configured_pack_dir_with_environment(&source.config, environment),
            data_directory: configured_data_dir_with_environment(&source.config, environment),
            app_policy: AppDisablePolicy::from_bundle_ids(
                source.config.apps.disable_bundle_ids.clone(),
                source.config.apps.disable_auto_bundle_ids.clone(),
            ),
            source_path: source.path,
        })
    }

    pub fn engine_source_description(&self) -> String {
        if self.data_directory.is_some() {
            "data_dir".to_owned()
        } else if self.secondary_language == "uk" {
            "embedded".to_owned()
        } else {
            format!("pack:{}", self.secondary_language)
        }
    }

    pub fn selected_pack_path(&self) -> Option<PathBuf> {
        let pack_directory = self.pack_directory.as_ref()?;
        Some(pack_directory.join(&self.secondary_language))
    }

    pub fn load_language_bundle(&self) -> Result<LanguageBundle, String> {
        if let Some(data_directory) = self.data_directory.as_deref() {
            return LanguageBundle::from_data_dir(data_directory).map_err(|error| {
                format!("load data directory {}: {error}", data_directory.display())
            });
        }
        if self.secondary_language == "uk" {
            return LanguageBundle::embedded()
                .map_err(|error| format!("load embedded data: {error}"));
        }
        let pack_path = self.selected_pack_path().ok_or_else(|| {
            format!(
                "no pack directory resolved for secondary language '{}'",
                self.secondary_language
            )
        })?;
        LanguageBundle::from_secondary_pack_dir(&pack_path)
            .map_err(|error| format!("load pack {}: {error}", pack_path.display()))
    }

    pub fn resolve_input_policy(&self, facts: &HostSurfaceFacts) -> HostInputPolicy {
        self.resolve_input_policy_view(&facts.as_view())
    }

    pub fn resolve_input_policy_view(&self, facts: &HostSurfaceFactsView<'_>) -> HostInputPolicy {
        if facts.secure_input {
            return HostInputPolicy {
                disable_automatic_processing: true,
                disable_manual_conversion: true,
                terminal_surface: false,
                reason: HostInputPolicyReason::SecureInput,
            };
        }

        if facts.bundle_id.is_some_and(is_terminal_bundle_id) {
            return terminal_policy(HostInputPolicyReason::TerminalBundle);
        }

        if facts.is_terminal_surface() {
            return terminal_policy(HostInputPolicyReason::TerminalSurface);
        }

        if let Some(bundle_id) = facts.bundle_id {
            if self.app_policy.disables_bundle(bundle_id) {
                return HostInputPolicy {
                    disable_automatic_processing: true,
                    disable_manual_conversion: true,
                    terminal_surface: false,
                    reason: HostInputPolicyReason::DisabledBundle,
                };
            }
            if self.app_policy.disables_automatic_processing(bundle_id) {
                return HostInputPolicy {
                    disable_automatic_processing: true,
                    disable_manual_conversion: false,
                    terminal_surface: false,
                    reason: HostInputPolicyReason::AutomaticProcessingDisabledBundle,
                };
            }
        }

        HostInputPolicy::default()
    }
}

impl HostEnvironment {
    pub fn from_process() -> Self {
        Self {
            config_path: env::var_os("TYPEFLOW_CONFIG").map(PathBuf::from),
            data_directory: env::var_os("TYPEFLOW_DATA_DIR").map(PathBuf::from),
            pack_directory: env::var_os("TYPEFLOW_PACK_DIR").map(PathBuf::from),
            home: env::var_os("HOME").map(PathBuf::from),
        }
    }
}

impl AppDisablePolicy {
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        let config: Config = toml::from_str(text)?;
        Ok(Self::from_bundle_ids(
            config.apps.disable_bundle_ids,
            config.apps.disable_auto_bundle_ids,
        ))
    }

    pub fn from_bundle_ids(
        disable_bundle_ids: impl IntoIterator<Item = String>,
        disable_auto_bundle_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        let disable_bundle_ids = normalized_bundle_ids(disable_bundle_ids);
        let mut disable_auto_bundle_ids = normalized_bundle_ids(disable_auto_bundle_ids);
        disable_auto_bundle_ids.retain(|bundle_id| !disable_bundle_ids.contains(bundle_id));

        Self {
            disable_bundle_ids,
            disable_auto_bundle_ids,
        }
    }

    pub fn disables_bundle(&self, bundle_id: &str) -> bool {
        self.disable_bundle_ids.contains(bundle_id)
    }

    pub fn disables_automatic_processing(&self, bundle_id: &str) -> bool {
        self.disable_bundle_ids.contains(bundle_id)
            || self.disable_auto_bundle_ids.contains(bundle_id)
    }

    pub fn disable_bundle_count(&self) -> usize {
        self.disable_bundle_ids.len()
    }

    pub fn disable_auto_bundle_count(&self) -> usize {
        self.disable_auto_bundle_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.disable_bundle_ids.is_empty() && self.disable_auto_bundle_ids.is_empty()
    }
}

impl HostSurfaceFacts {
    fn as_view(&self) -> HostSurfaceFactsView<'_> {
        HostSurfaceFactsView {
            secure_input: self.secure_input,
            bundle_id: self.bundle_id.as_deref(),
            application_name: self.application_name.as_deref(),
            input_client_class: self.input_client_class.as_deref(),
            focused_element_role: self.focused_element_role.as_deref(),
            focused_element_subrole: self.focused_element_subrole.as_deref(),
            focused_element_role_description: self.focused_element_role_description.as_deref(),
            focused_element_identifier: self.focused_element_identifier.as_deref(),
            focused_element_description: self.focused_element_description.as_deref(),
            focused_window_title: self.focused_window_title.as_deref(),
        }
    }
}

impl HostSurfaceFactsView<'_> {
    fn is_terminal_surface(&self) -> bool {
        [
            self.input_client_class,
            self.focused_element_role,
            self.focused_element_subrole,
            self.focused_element_role_description,
            self.focused_element_identifier,
            self.focused_element_description,
        ]
        .into_iter()
        .flatten()
        .any(contains_terminal_surface_marker)
    }
}

fn terminal_policy(reason: HostInputPolicyReason) -> HostInputPolicy {
    HostInputPolicy {
        disable_automatic_processing: true,
        disable_manual_conversion: true,
        terminal_surface: true,
        reason,
    }
}

fn is_terminal_bundle_id(bundle_id: &str) -> bool {
    TERMINAL_BUNDLE_IDS
        .iter()
        .any(|terminal_bundle_id| bundle_id.eq_ignore_ascii_case(terminal_bundle_id))
}

fn contains_terminal_surface_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.contains("pseudo terminal") {
        return true;
    }

    lower
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|token| !token.is_empty() && TERMINAL_SURFACE_MARKERS.contains(&token))
}

fn normalized_bundle_ids(bundle_ids: impl IntoIterator<Item = String>) -> HashSet<String> {
    bundle_ids
        .into_iter()
        .filter_map(|bundle_id| {
            let trimmed = bundle_id.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .collect()
}

pub fn read_path(path: &Path) -> Result<Config, String> {
    let bytes =
        fs::read_to_string(path).map_err(|e| format!("read config {}: {e}", path.display()))?;
    toml::from_str::<Config>(&bytes).map_err(|e| format!("parse config {}: {e}", path.display()))
}

pub fn home_default() -> Option<PathBuf> {
    home_default_with_environment(&HostEnvironment::from_process())
}

pub fn home_default_with_environment(environment: &HostEnvironment) -> Option<PathBuf> {
    Some(
        environment
            .home
            .as_ref()?
            .join(".config/typeflow/config.toml"),
    )
}

pub fn default_pack_dir() -> Option<PathBuf> {
    default_pack_dir_with_environment(&HostEnvironment::from_process())
}

pub fn default_pack_dir_with_environment(environment: &HostEnvironment) -> Option<PathBuf> {
    let home = environment.home.as_ref()?;

    #[cfg(target_os = "macos")]
    {
        Some(
            home.join("Library")
                .join("Application Support")
                .join("Typeflow")
                .join("packs"),
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        Some(home.join(".local/share/typeflow/packs"))
    }
}

pub fn configured_data_dir(config: &Config) -> Option<PathBuf> {
    configured_data_dir_with_environment(config, &HostEnvironment::from_process())
}

pub fn configured_data_dir_with_environment(
    config: &Config,
    environment: &HostEnvironment,
) -> Option<PathBuf> {
    environment
        .data_directory
        .clone()
        .or_else(|| config.data.directory.clone())
}

pub fn configured_pack_dir(config: &Config) -> Option<PathBuf> {
    configured_pack_dir_with_environment(config, &HostEnvironment::from_process())
}

pub fn configured_pack_dir_with_environment(
    config: &Config,
    environment: &HostEnvironment,
) -> Option<PathBuf> {
    environment
        .pack_directory
        .clone()
        .or_else(|| config.packs.directory.clone())
        .or_else(|| default_pack_dir_with_environment(environment))
}

pub fn normalized_secondary_id(config: &Config) -> String {
    let id = config.language.secondary.trim();
    if id.is_empty() {
        "uk".to_owned()
    } else {
        id.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppDisablePolicy, Config, ConfigSource, HostEnvironment, HostInputPolicyReason,
        HostSurfaceFacts, ResolvedHostConfig, configured_data_dir_with_environment,
        configured_pack_dir_with_environment, default_pack_dir_with_environment,
        home_default_with_environment, normalized_secondary_id,
    };

    #[test]
    fn parses_app_disable_policy_from_full_config() {
        let policy = AppDisablePolicy::from_toml(
            r#"
[engine]
min_token_len = 4

[apps]
disable_bundle_ids = [
    "dev.zed.Zed",
    " com.apple.Terminal ",
    "",
]

disable_auto_bundle_ids = [
    "com.tinyspeck.slackmacgap",
    "dev.zed.Zed",
]
"#,
        )
        .unwrap();

        assert_eq!(policy.disable_bundle_count(), 2);
        assert_eq!(policy.disable_auto_bundle_count(), 1);
        assert!(policy.disables_bundle("dev.zed.Zed"));
        assert!(policy.disables_bundle("com.apple.Terminal"));
        assert!(policy.disables_automatic_processing("dev.zed.Zed"));
        assert!(policy.disables_automatic_processing("com.tinyspeck.slackmacgap"));
        assert!(!policy.disables_bundle("com.tinyspeck.slackmacgap"));
        assert!(!policy.disables_automatic_processing("com.apple.TextEdit"));
    }

    #[test]
    fn legacy_app_disable_alias_maps_to_auto_disable() {
        let policy = AppDisablePolicy::from_toml(
            r#"
[apps]
exclude_bundle_ids = ["dev.zed.Zed"]
"#,
        )
        .unwrap();

        assert!(!policy.disables_bundle("dev.zed.Zed"));
        assert!(policy.disables_automatic_processing("dev.zed.Zed"));
    }

    #[test]
    fn missing_apps_section_means_no_app_policy() {
        let policy = AppDisablePolicy::from_toml("[language]\nsecondary = \"uk\"\n").unwrap();

        assert!(policy.is_empty());
    }

    #[test]
    fn terminal_bundle_policy_disables_auto_and_manual() {
        let resolved = ResolvedHostConfig::from_source(
            ConfigSource {
                config: Config::default(),
                path: None,
            },
            &HostEnvironment::default(),
        )
        .unwrap();
        let policy = resolved.resolve_input_policy(&HostSurfaceFacts {
            bundle_id: Some("com.googlecode.iterm2".to_owned()),
            ..HostSurfaceFacts::default()
        });

        assert_eq!(policy.reason, HostInputPolicyReason::TerminalBundle);
        assert!(policy.disable_automatic_processing);
        assert!(policy.disable_manual_conversion);
        assert!(policy.terminal_surface);
    }

    #[test]
    fn terminal_surface_policy_disables_auto_and_manual_inside_other_apps() {
        let resolved = ResolvedHostConfig::from_source(
            ConfigSource {
                config: Config::default(),
                path: None,
            },
            &HostEnvironment::default(),
        )
        .unwrap();
        let policy = resolved.resolve_input_policy(&HostSurfaceFacts {
            bundle_id: Some("dev.zed.Zed".to_owned()),
            focused_element_identifier: Some("workspace-terminal-panel".to_owned()),
            ..HostSurfaceFacts::default()
        });

        assert_eq!(policy.reason, HostInputPolicyReason::TerminalSurface);
        assert!(policy.disable_automatic_processing);
        assert!(policy.disable_manual_conversion);
        assert!(policy.terminal_surface);
    }

    #[test]
    fn terminal_surface_policy_ignores_low_signal_titles() {
        let resolved = ResolvedHostConfig::from_source(
            ConfigSource {
                config: Config::default(),
                path: None,
            },
            &HostEnvironment::default(),
        )
        .unwrap();
        let policy = resolved.resolve_input_policy(&HostSurfaceFacts {
            bundle_id: Some("com.apple.TextEdit".to_owned()),
            focused_window_title: Some("terminal notes".to_owned()),
            focused_element_description: Some("shellfish recipe".to_owned()),
            ..HostSurfaceFacts::default()
        });

        assert_eq!(policy.reason, HostInputPolicyReason::Normal);
        assert!(!policy.disable_automatic_processing);
    }

    #[test]
    fn auto_disabled_bundle_still_allows_manual_conversion() {
        let config = toml::from_str::<Config>(
            r#"
[apps]
disable_auto_bundle_ids = ["dev.zed.Zed"]
"#,
        )
        .unwrap();
        let resolved = ResolvedHostConfig::from_source(
            ConfigSource { config, path: None },
            &HostEnvironment::default(),
        )
        .unwrap();
        let policy = resolved.resolve_input_policy(&HostSurfaceFacts {
            bundle_id: Some("dev.zed.Zed".to_owned()),
            ..HostSurfaceFacts::default()
        });

        assert_eq!(
            policy.reason,
            HostInputPolicyReason::AutomaticProcessingDisabledBundle
        );
        assert!(policy.disable_automatic_processing);
        assert!(!policy.disable_manual_conversion);
        assert!(!policy.terminal_surface);
    }

    #[test]
    fn secure_input_precedes_every_other_policy() {
        let resolved = ResolvedHostConfig::from_source(
            ConfigSource {
                config: Config::default(),
                path: None,
            },
            &HostEnvironment::default(),
        )
        .unwrap();
        let policy = resolved.resolve_input_policy(&HostSurfaceFacts {
            secure_input: true,
            bundle_id: Some("com.googlecode.iterm2".to_owned()),
            ..HostSurfaceFacts::default()
        });

        assert_eq!(policy.reason, HostInputPolicyReason::SecureInput);
        assert!(policy.disable_automatic_processing);
        assert!(policy.disable_manual_conversion);
        assert!(!policy.terminal_surface);
    }

    #[test]
    fn config_defaults_match_embedded_secondary() {
        let config = Config::default();

        assert_eq!(normalized_secondary_id(&config), "uk");
        assert_eq!(config.engine.min_token_len, 4);
    }

    #[test]
    fn environment_overrides_data_and_pack_dirs() {
        let config = toml::from_str::<Config>(
            r#"
[packs]
directory = "/from/config/packs"

[data]
directory = "/from/config/data"
"#,
        )
        .unwrap();
        let environment = HostEnvironment {
            data_directory: Some("/from/env/data".into()),
            pack_directory: Some("/from/env/packs".into()),
            ..HostEnvironment::default()
        };

        assert_eq!(
            configured_data_dir_with_environment(&config, &environment),
            Some("/from/env/data".into())
        );
        assert_eq!(
            configured_pack_dir_with_environment(&config, &environment),
            Some("/from/env/packs".into())
        );
    }

    #[test]
    fn resolved_host_config_trims_language_and_builds_app_policy() {
        let config = toml::from_str::<Config>(
            r#"
[language]
secondary = " uk "

[apps]
disable_auto_bundle_ids = ["dev.zed.Zed"]
"#,
        )
        .unwrap();
        let resolved = ResolvedHostConfig::from_source(
            ConfigSource {
                config,
                path: Some("/tmp/typeflow.toml".into()),
            },
            &HostEnvironment::default(),
        )
        .unwrap();

        assert_eq!(resolved.secondary_language, "uk");
        assert_eq!(resolved.source_path, Some("/tmp/typeflow.toml".into()));
        assert!(
            resolved
                .app_policy
                .disables_automatic_processing("dev.zed.Zed")
        );
        assert_eq!(resolved.engine_source_description(), "embedded");
    }

    #[test]
    fn default_paths_derive_from_home() {
        let environment = HostEnvironment {
            home: Some("/Users/example".into()),
            ..HostEnvironment::default()
        };

        assert_eq!(
            home_default_with_environment(&environment),
            Some("/Users/example/.config/typeflow/config.toml".into())
        );
        assert!(
            default_pack_dir_with_environment(&environment)
                .unwrap()
                .ends_with("Typeflow/packs")
        );
    }
}
