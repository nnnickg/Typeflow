#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock};

use typeflow_core::data::LanguageBundle;
use typeflow_core::{
    Engine, EngineConfig, InputEvent, Layout, LetterEvent, ObservationAction, PhysicalKey,
};
use typeflow_host_config::{
    Config, ConfigSource, HostEnvironment, HostInputPolicy, HostInputPolicyReason,
    HostSurfaceFactsView, ResolvedHostConfig,
};

pub const TF_EVENT_LETTER: u8 = 0;
pub const TF_EVENT_BACKSPACE: u8 = 1;
pub const TF_EVENT_END_TOKEN: u8 = 2;
pub const TF_EVENT_LITERAL: u8 = 3;

pub const TF_MOD_SHIFT: u8 = 0x01;
pub const TF_MOD_CONTROL: u8 = 0x02;
pub const TF_MOD_OPTION: u8 = 0x04;
pub const TF_MOD_COMMAND: u8 = 0x08;

pub const TF_CONTEXT_SECURE_INPUT: u32 = 0x01;
pub const TF_CONTEXT_AUTOMATIC_PROCESSING_DISABLED: u32 = 0x02;
pub const TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED: u32 = 0x04;

pub const TF_HOST_POLICY_SECURE_INPUT: u32 = 0x01;
pub const TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED: u32 = 0x02;
pub const TF_HOST_POLICY_MANUAL_SWITCH_DISABLED: u32 = 0x04;
pub const TF_HOST_POLICY_TERMINAL_SURFACE: u32 = 0x08;

pub const TF_HOST_POLICY_REASON_NORMAL: u8 = 0;
pub const TF_HOST_POLICY_REASON_SECURE_INPUT: u8 = 1;
pub const TF_HOST_POLICY_REASON_TERMINAL_BUNDLE: u8 = 2;
pub const TF_HOST_POLICY_REASON_TERMINAL_SURFACE: u8 = 3;
pub const TF_HOST_POLICY_REASON_DISABLED_BUNDLE: u8 = 4;
pub const TF_HOST_POLICY_REASON_AUTOMATIC_PROCESSING_DISABLED_BUNDLE: u8 = 5;
pub const TF_HOST_POLICY_REASON_UNAVAILABLE_HOST_CONFIG: u8 = 255;

pub const TF_OBSERVATION_NONE: u8 = 0;
pub const TF_OBSERVATION_SWITCH_FUTURE_LAYOUT: u8 = 1;
pub const TF_OBSERVATION_RESET_TOKEN: u8 = 2;

pub const TF_LAYOUT_ENGLISH: u8 = 0;
pub const TF_LAYOUT_SECONDARY: u8 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TfEvent {
    pub tag: u8,
    pub physical: u8,
    pub modifiers: u8,
    pub codepoint: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TfEngineConfig {
    pub min_token_len: usize,
    pub max_token_len: usize,
    pub confidence_margin: f32,
    pub dict_exact_weight: f32,
    pub dict_prefix_weight: f32,
    pub ngram_only_confidence_margin: f32,
    pub bigram_weight: f32,
    pub trigram_weight: f32,
    pub length_normalize: u8,
    pub disable_on_internal_caps: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TfObservation {
    pub tag: u8,
    pub layout: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TfHostSurfaceFacts {
    pub secure_input: u8,
    pub bundle_id_utf8: *const c_char,
    pub application_name_utf8: *const c_char,
    pub input_client_class_utf8: *const c_char,
    pub focused_element_role_utf8: *const c_char,
    pub focused_element_subrole_utf8: *const c_char,
    pub focused_element_role_description_utf8: *const c_char,
    pub focused_element_identifier_utf8: *const c_char,
    pub focused_element_description_utf8: *const c_char,
    pub focused_element_context_utf8: *const c_char,
    pub focused_window_title_utf8: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TfHostInputPolicy {
    pub flags: u32,
    pub reason: u8,
}

pub struct TfEngine {
    engine: Engine,
    pending_replacement: Option<PendingReplacement>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingReplacement {
    delete_count: usize,
    text: String,
    inverse_text: String,
}

pub struct TfHostConfig {
    config: ResolvedHostConfig,
    source_path: Option<CString>,
    secondary_language: CString,
    pack_directory: Option<CString>,
    data_directory: Option<CString>,
    engine_source: CString,
    macos_english_input_source_id: Option<CString>,
    macos_secondary_input_source_id: Option<CString>,
}

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

static LANGUAGE_BUNDLE_CACHE: OnceLock<Mutex<HashMap<String, Arc<LanguageBundle>>>> =
    OnceLock::new();

fn set_last_error(message: impl Into<String>) {
    let message = message.into().replace('\0', "\\0");
    let error = CString::new(message).ok().or_else(|| {
        CString::from_vec_with_nul(b"typeflow error contained invalid bytes\0".to_vec()).ok()
    });
    if let Some(error) = error {
        LAST_ERROR.with(|last_error| {
            *last_error.borrow_mut() = Some(error);
        });
    }
}

fn clear_last_error() {
    LAST_ERROR.with(|last_error| {
        *last_error.borrow_mut() = None;
    });
}

fn ffi_guard<T>(fallback: T, call: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(value) => value,
        Err(payload) => {
            set_last_error(panic_message(payload.as_ref()));
            fallback
        }
    }
}

fn ffi_guard_void(call: impl FnOnce()) {
    ffi_guard((), call);
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        format!("panic crossed Typeflow FFI boundary: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("panic crossed Typeflow FFI boundary: {message}")
    } else {
        "panic crossed Typeflow FFI boundary".to_owned()
    }
}

fn language_bundle_cache() -> &'static Mutex<HashMap<String, Arc<LanguageBundle>>> {
    LANGUAGE_BUNDLE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_language_bundle(
    key: String,
    load: impl FnOnce() -> Result<LanguageBundle, String>,
) -> Result<Arc<LanguageBundle>, String> {
    if let Some(bundle) = language_bundle_cache()
        .lock()
        .map_err(|_| "language bundle cache lock poisoned".to_owned())?
        .get(&key)
        .cloned()
    {
        return Ok(bundle);
    }

    let loaded = Arc::new(load()?);
    let mut cache = language_bundle_cache()
        .lock()
        .map_err(|_| "language bundle cache lock poisoned".to_owned())?;
    let bundle = match cache.entry(key) {
        Entry::Occupied(entry) => Arc::clone(entry.get()),
        Entry::Vacant(entry) => Arc::clone(entry.insert(loaded)),
    };
    Ok(bundle)
}

fn embedded_bundle() -> Result<Arc<LanguageBundle>, String> {
    LanguageBundle::embedded_shared().map_err(|error| format!("load embedded data: {error}"))
}

fn data_dir_bundle(path: &Path) -> Result<Arc<LanguageBundle>, String> {
    let key = format!("data:{}", path.display());
    cached_language_bundle(key, || {
        LanguageBundle::from_data_dir(path)
            .map_err(|error| format!("load data directory {}: {error}", path.display()))
    })
}

fn pack_dir_bundle(path: &Path) -> Result<Arc<LanguageBundle>, String> {
    let key = format!("pack:{}", path.display());
    cached_language_bundle(key, || {
        LanguageBundle::from_secondary_pack_dir(path)
            .map_err(|error| format!("load pack {}: {error}", path.display()))
    })
}

fn host_config_bundle(config: &ResolvedHostConfig) -> Result<Arc<LanguageBundle>, String> {
    if let Some(data_directory) = config.data_directory.as_deref() {
        return data_dir_bundle(data_directory);
    }
    if config.secondary_language == "uk" {
        return embedded_bundle();
    }
    let pack_path = config.selected_pack_path().ok_or_else(|| {
        format!(
            "no pack directory resolved for secondary language '{}'",
            config.secondary_language
        )
    })?;
    pack_dir_bundle(&pack_path)
}

#[unsafe(no_mangle)]
pub extern "C" fn typeflow_last_error_message() -> *const c_char {
    ffi_guard(std::ptr::null(), || {
        LAST_ERROR.with(|last_error| {
            last_error
                .borrow()
                .as_ref()
                .map(|error| error.as_ptr())
                .unwrap_or(std::ptr::null())
        })
    })
}

impl TfObservation {
    fn write(&mut self, action: ObservationAction) {
        self.tag = TF_OBSERVATION_NONE;
        self.layout = TF_LAYOUT_ENGLISH;

        match action {
            ObservationAction::None => {}
            ObservationAction::SwitchFutureLayout(layout) => {
                self.tag = TF_OBSERVATION_SWITCH_FUTURE_LAYOUT;
                self.layout = layout_to_u8(layout);
            }
            ObservationAction::ResetToken => {
                self.tag = TF_OBSERVATION_RESET_TOKEN;
            }
        }
    }
}

fn layout_to_u8(layout: Layout) -> u8 {
    match layout {
        Layout::English => TF_LAYOUT_ENGLISH,
        Layout::Secondary => TF_LAYOUT_SECONDARY,
    }
}

fn opposite_layout(layout: Layout) -> Layout {
    match layout {
        Layout::English => Layout::Secondary,
        Layout::Secondary => Layout::English,
    }
}

impl TfEngine {
    fn capture_replacement(&self, layout: Layout) -> Option<PendingReplacement> {
        let delete_count = self.engine.token_len();
        if delete_count == 0 {
            return None;
        }

        let candidates = self.engine.token_candidates();
        let text = candidates.get(layout).to_owned();
        if text.is_empty() {
            return None;
        }
        let inverse_text = candidates.get(opposite_layout(layout)).to_owned();

        Some(PendingReplacement {
            delete_count,
            text,
            inverse_text,
        })
    }

    fn set_pending_replacement(&mut self, layout: Layout) {
        self.pending_replacement = self.capture_replacement(layout);
    }

    fn clear_pending_replacement(&mut self) {
        self.pending_replacement = None;
    }
}

fn decode_event(event: TfEvent) -> Option<InputEvent> {
    if event.modifiers & (TF_MOD_CONTROL | TF_MOD_OPTION | TF_MOD_COMMAND) != 0 {
        return Some(InputEvent::HostBypass);
    }

    match event.tag {
        TF_EVENT_BACKSPACE => Some(InputEvent::Backspace),
        TF_EVENT_END_TOKEN => Some(InputEvent::EndToken),
        TF_EVENT_LITERAL => char::from_u32(event.codepoint).map(InputEvent::Literal),
        TF_EVENT_LETTER => {
            let physical = PhysicalKey::from_index(event.physical)?;
            Some(InputEvent::Letter(LetterEvent {
                physical_key: physical,
                shift: event.modifiers & TF_MOD_SHIFT != 0,
            }))
        }
        _ => None,
    }
}

fn default_ffi_config() -> TfEngineConfig {
    engine_config_to_ffi(EngineConfig::default())
}

fn engine_config_to_ffi(config: EngineConfig) -> TfEngineConfig {
    TfEngineConfig {
        min_token_len: config.min_token_len,
        max_token_len: config.max_token_len,
        confidence_margin: config.confidence_margin,
        dict_exact_weight: config.dict_exact_weight,
        dict_prefix_weight: config.dict_prefix_weight,
        ngram_only_confidence_margin: config.ngram_only_confidence_margin,
        bigram_weight: config.bigram_weight,
        trigram_weight: config.trigram_weight,
        length_normalize: u8::from(config.length_normalize),
        disable_on_internal_caps: u8::from(config.disable_on_internal_caps),
    }
}

fn engine_config_from_ffi(config: TfEngineConfig) -> Option<EngineConfig> {
    let engine_config = EngineConfig {
        min_token_len: config.min_token_len,
        max_token_len: config.max_token_len,
        confidence_margin: config.confidence_margin,
        dict_exact_weight: config.dict_exact_weight,
        dict_prefix_weight: config.dict_prefix_weight,
        ngram_only_confidence_margin: config.ngram_only_confidence_margin,
        bigram_weight: config.bigram_weight,
        trigram_weight: config.trigram_weight,
        length_normalize: config.length_normalize != 0,
        disable_on_internal_caps: config.disable_on_internal_caps != 0,
    };

    engine_config.validate().ok()?;
    Some(engine_config)
}

fn new_engine(bundle: Arc<LanguageBundle>, config: TfEngineConfig) -> *mut TfEngine {
    let Some(config) = engine_config_from_ffi(config) else {
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(TfEngine {
        engine: Engine::with_shared_bundle(config, bundle),
        pending_replacement: None,
    }))
}

fn new_engine_or_error(bundle: Arc<LanguageBundle>, config: TfEngineConfig) -> *mut TfEngine {
    let engine = new_engine(bundle, config);
    if engine.is_null() {
        set_last_error("invalid engine config");
    } else {
        clear_last_error();
    }
    engine
}

fn host_config_to_ffi(config: ResolvedHostConfig) -> Option<TfHostConfig> {
    let source_path = optional_path_cstring(config.source_path.as_deref())?;
    let pack_directory = optional_path_cstring(config.pack_directory.as_deref())?;
    let data_directory = optional_path_cstring(config.data_directory.as_deref())?;
    let secondary_language = CString::new(config.secondary_language.as_str()).ok()?;
    let engine_source = CString::new(config.engine_source_description()).ok()?;
    let macos_english_input_source_id = optional_string_cstring(
        config
            .macos_input_sources
            .english_input_source_id
            .as_deref(),
    )?;
    let macos_secondary_input_source_id = optional_string_cstring(
        config
            .macos_input_sources
            .secondary_input_source_id
            .as_deref(),
    )?;

    Some(TfHostConfig {
        config,
        source_path,
        secondary_language,
        pack_directory,
        data_directory,
        engine_source,
        macos_english_input_source_id,
        macos_secondary_input_source_id,
    })
}

fn optional_path_cstring(path: Option<&Path>) -> Option<Option<CString>> {
    match path {
        Some(path) => path_cstring(path).map(Some),
        None => Some(None),
    }
}

fn path_cstring(path: &Path) -> Option<CString> {
    CString::new(path.to_string_lossy().as_bytes()).ok()
}

fn optional_string_cstring(value: Option<&str>) -> Option<Option<CString>> {
    match value {
        Some(value) => CString::new(value).ok().map(Some),
        None => Some(None),
    }
}

fn host_context_from_flags(flags: u32) -> typeflow_core::HostContext {
    typeflow_core::HostContext {
        secure_input: flags & TF_CONTEXT_SECURE_INPUT != 0,
        automatic_processing_disabled: flags & TF_CONTEXT_AUTOMATIC_PROCESSING_DISABLED != 0,
        automatic_switching_disabled: flags & TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED != 0,
    }
}

unsafe fn host_surface_facts_from_ffi<'a>(facts: TfHostSurfaceFacts) -> HostSurfaceFactsView<'a> {
    HostSurfaceFactsView {
        secure_input: facts.secure_input != 0,
        bundle_id: unsafe { borrowed_c_string(facts.bundle_id_utf8) },
        application_name: unsafe { borrowed_c_string(facts.application_name_utf8) },
        input_client_class: unsafe { borrowed_c_string(facts.input_client_class_utf8) },
        focused_element_role: unsafe { borrowed_c_string(facts.focused_element_role_utf8) },
        focused_element_subrole: unsafe { borrowed_c_string(facts.focused_element_subrole_utf8) },
        focused_element_role_description: unsafe {
            borrowed_c_string(facts.focused_element_role_description_utf8)
        },
        focused_element_identifier: unsafe {
            borrowed_c_string(facts.focused_element_identifier_utf8)
        },
        focused_element_description: unsafe {
            borrowed_c_string(facts.focused_element_description_utf8)
        },
        focused_element_context: unsafe { borrowed_c_string(facts.focused_element_context_utf8) },
        focused_window_title: unsafe { borrowed_c_string(facts.focused_window_title_utf8) },
    }
}

unsafe fn borrowed_c_string<'a>(value_utf8: *const c_char) -> Option<&'a str> {
    let value = unsafe { c_str(value_utf8) }?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn host_input_policy_to_ffi(policy: HostInputPolicy) -> TfHostInputPolicy {
    let mut flags = 0;
    if policy.reason == HostInputPolicyReason::SecureInput {
        flags |= TF_HOST_POLICY_SECURE_INPUT;
    }
    if policy.disable_automatic_processing {
        flags |= TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED;
    }
    if policy.disable_manual_switch {
        flags |= TF_HOST_POLICY_MANUAL_SWITCH_DISABLED;
    }
    if policy.terminal_surface {
        flags |= TF_HOST_POLICY_TERMINAL_SURFACE;
    }

    TfHostInputPolicy {
        flags,
        reason: host_input_policy_reason_to_u8(policy.reason),
    }
}

fn unavailable_host_config_policy() -> TfHostInputPolicy {
    TfHostInputPolicy {
        flags: TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED | TF_HOST_POLICY_MANUAL_SWITCH_DISABLED,
        reason: TF_HOST_POLICY_REASON_UNAVAILABLE_HOST_CONFIG,
    }
}

fn host_input_policy_reason_to_u8(reason: HostInputPolicyReason) -> u8 {
    match reason {
        HostInputPolicyReason::Normal => TF_HOST_POLICY_REASON_NORMAL,
        HostInputPolicyReason::SecureInput => TF_HOST_POLICY_REASON_SECURE_INPUT,
        HostInputPolicyReason::TerminalBundle => TF_HOST_POLICY_REASON_TERMINAL_BUNDLE,
        HostInputPolicyReason::TerminalSurface => TF_HOST_POLICY_REASON_TERMINAL_SURFACE,
        HostInputPolicyReason::DisabledBundle => TF_HOST_POLICY_REASON_DISABLED_BUNDLE,
        HostInputPolicyReason::AutomaticProcessingDisabledBundle => {
            TF_HOST_POLICY_REASON_AUTOMATIC_PROCESSING_DISABLED_BUNDLE
        }
    }
}

fn layout_from_u8(value: u8) -> Option<Layout> {
    match value {
        TF_LAYOUT_ENGLISH => Some(Layout::English),
        TF_LAYOUT_SECONDARY => Some(Layout::Secondary),
        _ => None,
    }
}

/// Allocates a new engine using the language bundle embedded into the library.
///
/// This is the normal constructor for release builds and the macOS observer agent.
/// Returns null if the embedded bundle fails to deserialize.
#[unsafe(no_mangle)]
pub extern "C" fn typeflow_engine_new_embedded() -> *mut TfEngine {
    ffi_guard(std::ptr::null_mut(), || {
        typeflow_engine_new_embedded_with_config(default_ffi_config())
    })
}

/// Allocates a new embedded engine with explicit runtime tuning.
///
/// Returns null if the embedded bundle fails to deserialize or the config contains
/// invalid numeric values.
#[unsafe(no_mangle)]
pub extern "C" fn typeflow_engine_new_embedded_with_config(
    config: TfEngineConfig,
) -> *mut TfEngine {
    ffi_guard(std::ptr::null_mut(), || match embedded_bundle() {
        Ok(bundle) => new_engine_or_error(bundle, config),
        Err(error) => {
            set_last_error(error);
            std::ptr::null_mut()
        }
    })
}

/// Allocates a new engine, loading the language bundle from the directory at `data_dir_utf8`.
/// `data_dir_utf8` must point to a NUL-terminated UTF-8 path. Returns null on failure.
///
/// # Safety
///
/// `data_dir_utf8` must be either null or a valid pointer to a NUL-terminated UTF-8 C string
/// that remains alive for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_new_from_data_dir(
    data_dir_utf8: *const c_char,
) -> *mut TfEngine {
    ffi_guard(std::ptr::null_mut(), || unsafe {
        typeflow_engine_new_from_data_dir_with_config(data_dir_utf8, default_ffi_config())
    })
}

/// Allocates a new engine from `data_dir_utf8` with explicit runtime tuning.
///
/// Returns null if path decoding, data loading, or config validation fails.
///
/// # Safety
///
/// `data_dir_utf8` must be either null or a valid pointer to a NUL-terminated UTF-8 C string
/// that remains alive for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_new_from_data_dir_with_config(
    data_dir_utf8: *const c_char,
    config: TfEngineConfig,
) -> *mut TfEngine {
    ffi_guard(std::ptr::null_mut(), || {
        let Some(path) = (unsafe { c_path(data_dir_utf8) }) else {
            return std::ptr::null_mut();
        };
        match data_dir_bundle(&path) {
            Ok(bundle) => new_engine_or_error(bundle, config),
            Err(error) => {
                set_last_error(error);
                std::ptr::null_mut()
            }
        }
    })
}

/// Allocates a new engine using embedded English plus the secondary pack in `pack_dir_utf8`.
/// The pack directory must contain `pack.toml`, `ngrams.bin`, and `dict.fst`.
///
/// # Safety
///
/// `pack_dir_utf8` must be either null or a valid pointer to a NUL-terminated UTF-8 C string
/// that remains alive for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_new_from_pack_dir(
    pack_dir_utf8: *const c_char,
) -> *mut TfEngine {
    ffi_guard(std::ptr::null_mut(), || unsafe {
        typeflow_engine_new_from_pack_dir_with_config(pack_dir_utf8, default_ffi_config())
    })
}

/// Allocates a new engine from `pack_dir_utf8` with explicit runtime tuning.
///
/// Returns null if path decoding, pack loading, or config validation fails.
///
/// # Safety
///
/// `pack_dir_utf8` must be either null or a valid pointer to a NUL-terminated UTF-8 C string
/// that remains alive for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_new_from_pack_dir_with_config(
    pack_dir_utf8: *const c_char,
    config: TfEngineConfig,
) -> *mut TfEngine {
    ffi_guard(std::ptr::null_mut(), || {
        let Some(path) = (unsafe { c_path(pack_dir_utf8) }) else {
            return std::ptr::null_mut();
        };
        match pack_dir_bundle(&path) {
            Ok(bundle) => new_engine_or_error(bundle, config),
            Err(error) => {
                set_last_error(error);
                std::ptr::null_mut()
            }
        }
    })
}

/// Allocates a new engine from a resolved host config.
///
/// Rust owns the engine-source decision: data directory wins, embedded
/// Ukrainian is used for `secondary = "uk"`, otherwise the selected external
/// pack is loaded from the resolved pack directory.
///
/// # Safety
///
/// `config` must be a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_new_from_host_config(
    config: *const TfHostConfig,
) -> *mut TfEngine {
    ffi_guard(std::ptr::null_mut(), || {
        let Some(config) = (unsafe { config.as_ref() }) else {
            set_last_error("typeflow_engine_new_from_host_config received a null config pointer");
            return std::ptr::null_mut();
        };
        let bundle = match host_config_bundle(&config.config) {
            Ok(bundle) => bundle,
            Err(error) => {
                set_last_error(format!(
                    "failed to load language data for {}: {}",
                    config.config.engine_source_description(),
                    error
                ));
                return std::ptr::null_mut();
            }
        };
        let engine = new_engine(bundle, engine_config_to_ffi(config.config.engine));
        if engine.is_null() {
            set_last_error("invalid engine config in resolved host config");
        } else {
            clear_last_error();
        }
        engine
    })
}

unsafe fn c_path(path_utf8: *const c_char) -> Option<PathBuf> {
    unsafe { c_str(path_utf8) }.map(PathBuf::from)
}

unsafe fn c_str<'a>(value_utf8: *const c_char) -> Option<&'a str> {
    if value_utf8.is_null() {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(value_utf8) };
    cstr.to_str().ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn typeflow_host_config_load() -> *mut TfHostConfig {
    ffi_guard(std::ptr::null_mut(), || {
        let config = match ResolvedHostConfig::load(None) {
            Ok(config) => config,
            Err(error) => {
                set_last_error(error);
                return std::ptr::null_mut();
            }
        };
        let Some(config) = host_config_to_ffi(config) else {
            set_last_error("host config contains a path or language id with an embedded NUL byte");
            return std::ptr::null_mut();
        };
        clear_last_error();
        Box::into_raw(Box::new(config))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn typeflow_host_config_load_defaults() -> *mut TfHostConfig {
    ffi_guard(std::ptr::null_mut(), || {
        let environment = HostEnvironment::from_process();
        let source = ConfigSource {
            config: Config::default(),
            path: None,
        };
        let config = match ResolvedHostConfig::from_source(source, &environment) {
            Ok(config) => config,
            Err(error) => {
                set_last_error(error);
                return std::ptr::null_mut();
            }
        };
        let Some(config) = host_config_to_ffi(config) else {
            set_last_error(
                "default host config contains a path or language id with an embedded NUL byte",
            );
            return std::ptr::null_mut();
        };
        clear_last_error();
        Box::into_raw(Box::new(config))
    })
}

/// Loads host config from caller-supplied environment values.
///
/// Null pointers mean "unset". This exists so hosts/tests can validate config
/// precedence without reimplementing config parsing outside Rust.
///
/// # Safety
///
/// Each non-null pointer must be a valid NUL-terminated UTF-8 C string that
/// remains alive for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_load_with_environment(
    config_path_utf8: *const c_char,
    home_utf8: *const c_char,
    data_dir_utf8: *const c_char,
    pack_dir_utf8: *const c_char,
) -> *mut TfHostConfig {
    ffi_guard(std::ptr::null_mut(), || {
        let explicit = unsafe { c_path(config_path_utf8) };
        let environment = HostEnvironment {
            config_path: None,
            data_directory: unsafe { c_path(data_dir_utf8) },
            pack_directory: unsafe { c_path(pack_dir_utf8) },
            home: unsafe { c_path(home_utf8) },
        };

        let config =
            match ResolvedHostConfig::load_with_environment(explicit.as_deref(), &environment) {
                Ok(config) => config,
                Err(error) => {
                    set_last_error(error);
                    return std::ptr::null_mut();
                }
            };
        let Some(config) = host_config_to_ffi(config) else {
            set_last_error("host config contains a path or language id with an embedded NUL byte");
            return std::ptr::null_mut();
        };
        clear_last_error();
        Box::into_raw(Box::new(config))
    })
}

/// Frees host config allocated by Typeflow.
///
/// # Safety
///
/// `config` must be null or a pointer returned by a Typeflow host-config
/// constructor that has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_free(config: *mut TfHostConfig) {
    ffi_guard_void(|| {
        if !config.is_null() {
            unsafe {
                drop(Box::from_raw(config));
            }
        }
    });
}

/// Writes the resolved engine config into `out_config`.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
/// `out_config` must be null or point to writable memory for one `TfEngineConfig`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_engine_config(
    config: *const TfHostConfig,
    out_config: *mut TfEngineConfig,
) {
    ffi_guard_void(|| {
        let Some(out) = (unsafe { out_config.as_mut() }) else {
            return;
        };
        *out = unsafe { config.as_ref() }
            .map(|config| engine_config_to_ffi(config.config.engine))
            .unwrap_or_else(default_ffi_config);
    });
}

/// Returns the config file path, or null when defaults were used.
///
/// The returned pointer is owned by `config` and remains valid until `config`
/// is freed.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_source_path(
    config: *const TfHostConfig,
) -> *const c_char {
    ffi_guard(std::ptr::null(), || {
        unsafe { config.as_ref() }
            .and_then(|config| config.source_path.as_ref())
            .map(|value| value.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

/// Returns the normalized secondary language id.
///
/// The returned pointer is owned by `config` and remains valid until `config`
/// is freed.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_secondary_language(
    config: *const TfHostConfig,
) -> *const c_char {
    ffi_guard(std::ptr::null(), || {
        unsafe { config.as_ref() }
            .map(|config| config.secondary_language.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

/// Returns the resolved pack directory, or null when none could be resolved.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_pack_directory(
    config: *const TfHostConfig,
) -> *const c_char {
    ffi_guard(std::ptr::null(), || {
        unsafe { config.as_ref() }
            .and_then(|config| config.pack_directory.as_ref())
            .map(|value| value.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

/// Returns the resolved data directory, or null when none is configured.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_data_directory(
    config: *const TfHostConfig,
) -> *const c_char {
    ffi_guard(std::ptr::null(), || {
        unsafe { config.as_ref() }
            .and_then(|config| config.data_directory.as_ref())
            .map(|value| value.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

/// Returns `embedded`, `data_dir`, or `pack:<id>`.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_engine_source(
    config: *const TfHostConfig,
) -> *const c_char {
    ffi_guard(std::ptr::null(), || {
        unsafe { config.as_ref() }
            .map(|config| config.engine_source.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

/// Returns the configured macOS English input-source id, or null when unset.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_macos_english_input_source_id(
    config: *const TfHostConfig,
) -> *const c_char {
    ffi_guard(std::ptr::null(), || {
        unsafe { config.as_ref() }
            .and_then(|config| config.macos_english_input_source_id.as_ref())
            .map(|value| value.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

/// Returns the configured macOS secondary input-source id, or null when unset.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_macos_secondary_input_source_id(
    config: *const TfHostConfig,
) -> *const c_char {
    ffi_guard(std::ptr::null(), || {
        unsafe { config.as_ref() }
            .and_then(|config| config.macos_secondary_input_source_id.as_ref())
            .map(|value| value.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

/// Returns 1 when `bundle_id_utf8` is fully disabled.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
/// `bundle_id_utf8` must be null or a valid NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_is_bundle_disabled(
    config: *const TfHostConfig,
    bundle_id_utf8: *const c_char,
) -> u8 {
    ffi_guard(0, || {
        let Some(config) = (unsafe { config.as_ref() }) else {
            return 0;
        };
        let Some(bundle_id) = (unsafe { c_str(bundle_id_utf8) }) else {
            return 0;
        };
        u8::from(config.config.app_policy.disables_bundle(bundle_id))
    })
}

/// Returns 1 when automatic processing is disabled for `bundle_id_utf8`.
///
/// Fully disabled bundles also disable automatic processing.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
/// `bundle_id_utf8` must be null or a valid NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_is_automatic_processing_disabled(
    config: *const TfHostConfig,
    bundle_id_utf8: *const c_char,
) -> u8 {
    ffi_guard(0, || {
        let Some(config) = (unsafe { config.as_ref() }) else {
            return 0;
        };
        let Some(bundle_id) = (unsafe { c_str(bundle_id_utf8) }) else {
            return 0;
        };
        u8::from(
            config
                .config
                .app_policy
                .disables_automatic_processing(bundle_id),
        )
    })
}

/// Returns the number of fully disabled bundle IDs.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_disabled_bundle_count(
    config: *const TfHostConfig,
) -> usize {
    ffi_guard(0, || {
        unsafe { config.as_ref() }
            .map(|config| config.config.app_policy.disable_bundle_count())
            .unwrap_or(0)
    })
}

/// Returns the number of bundle IDs with automatic processing disabled.
///
/// This count does not include fully disabled bundle IDs; use
/// `typeflow_host_config_disabled_bundle_count` for that list.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_auto_disabled_bundle_count(
    config: *const TfHostConfig,
) -> usize {
    ffi_guard(0, || {
        unsafe { config.as_ref() }
            .map(|config| config.config.app_policy.disable_auto_bundle_count())
            .unwrap_or(0)
    })
}

/// Resolves host-surface facts into Typeflow input policy.
///
/// Rust owns the policy decision; hosts only provide facts about the current
/// macOS surface.
///
/// # Safety
///
/// `config` must be null or a valid live Typeflow host-config pointer. Every
/// non-null pointer inside `facts` must point to a valid NUL-terminated UTF-8
/// string that remains alive for this call. `out_policy` must be null or point
/// to writable memory for one `TfHostInputPolicy`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_host_config_resolve_input_policy(
    config: *const TfHostConfig,
    facts: TfHostSurfaceFacts,
    out_policy: *mut TfHostInputPolicy,
) {
    ffi_guard_void(|| {
        let Some(out) = (unsafe { out_policy.as_mut() }) else {
            return;
        };
        *out = unavailable_host_config_policy();
        let Some(config) = (unsafe { config.as_ref() }) else {
            return;
        };

        let facts = unsafe { host_surface_facts_from_ffi(facts) };
        *out = host_input_policy_to_ffi(config.config.resolve_input_policy_view(&facts));
    });
}

/// Frees an engine pointer created by `typeflow_engine_new_embedded` or
/// `typeflow_engine_new_from_data_dir` / `typeflow_engine_new_from_pack_dir`.
///
/// Passing any other pointer is undefined behavior.
///
/// # Safety
///
/// `engine` must be null or a pointer returned by any Typeflow constructor
/// (`typeflow_engine_new_embedded`, `typeflow_engine_new_from_data_dir`,
/// `typeflow_engine_new_from_pack_dir`) that has not already been freed.
/// After this call, the pointer must not be used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_free(engine: *mut TfEngine) {
    ffi_guard_void(|| {
        if !engine.is_null() {
            unsafe {
                drop(Box::from_raw(engine));
            }
        }
    });
}

/// Clears the current token buffer without changing the active layout.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_reset_token(engine: *mut TfEngine) {
    ffi_guard_void(|| {
        if let Some(engine) = unsafe { engine.as_mut() } {
            engine.clear_pending_replacement();
            engine.engine.reset_token();
        }
    });
}

/// Resets both the active layout and the current token. Use when the host
/// detects an out-of-band layout change (Cmd+Space, manual switch, etc.) and
/// needs to re-sync the engine state.
///
/// `layout` must be one of `TF_LAYOUT_ENGLISH` or `TF_LAYOUT_SECONDARY`;
/// any other value is ignored.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by a Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_reset_layout(engine: *mut TfEngine, layout: u8) {
    ffi_guard_void(|| {
        let Some(engine) = (unsafe { engine.as_mut() }) else {
            return;
        };
        let Some(layout) = layout_from_u8(layout) else {
            return;
        };
        engine.clear_pending_replacement();
        engine.engine.reset_layout(layout);
    });
}

/// Sets host-level bypass context.
///
/// `TF_CONTEXT_SECURE_INPUT` means a password/secure field is active.
/// `TF_CONTEXT_AUTOMATIC_PROCESSING_DISABLED` means automatic processing is
/// disabled for the foreground app.
/// `TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED` means automatic layout switches
/// are disabled, but the engine still observes the current token.
/// Secure input and full automatic-processing disable return Keep/Bypass and
/// clear the token; automatic-switching disable keeps normal observation.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by a Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_set_host_context(engine: *mut TfEngine, flags: u32) {
    ffi_guard_void(|| {
        if let Some(engine) = unsafe { engine.as_mut() } {
            engine.clear_pending_replacement();
            engine
                .engine
                .set_host_context(host_context_from_flags(flags));
        }
    });
}

/// Forces the current token to the opposite side of the active language pair.
///
/// When a token is active, this captures a pending replacement snapshot before
/// resetting the token. The host can consume that snapshot with
/// `typeflow_engine_take_pending_replacement_utf8`.
///
/// # Safety
///
/// `engine` must be a valid live pointer returned by `typeflow_engine_new_embedded` or
/// any other Typeflow constructor. `out_observation` must point to writable memory for one
/// `TfObservation`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_force_switch_layout(
    engine: *mut TfEngine,
    out_observation: *mut TfObservation,
) {
    ffi_guard_void(|| {
        let Some(engine) = (unsafe { engine.as_mut() }) else {
            return;
        };
        let Some(out) = (unsafe { out_observation.as_mut() }) else {
            return;
        };
        out.write(ObservationAction::None);

        let target = opposite_layout(engine.engine.current_layout());
        engine.set_pending_replacement(target);
        let output = engine.engine.force_switch_layout();
        out.write(output.action);
    });
}

/// Returns the engine's current inferred layout.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_current_layout(engine: *mut TfEngine) -> u8 {
    ffi_guard(TF_LAYOUT_ENGLISH, || match unsafe { engine.as_ref() } {
        Some(engine) => layout_to_u8(engine.engine.current_layout()),
        None => TF_LAYOUT_ENGLISH,
    })
}

/// Returns the engine's current tracked token length.
///
/// This is the number of logical token characters currently tracked by the
/// engine, not a byte count.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_token_len(engine: *mut TfEngine) -> usize {
    ffi_guard(0, || match unsafe { engine.as_ref() } {
        Some(engine) => engine.engine.token_len(),
        None => 0,
    })
}

/// Returns the character count to delete for the pending replacement captured
/// by the last `TF_OBSERVATION_SWITCH_FUTURE_LAYOUT` action.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_pending_replacement_delete_count(
    engine: *mut TfEngine,
) -> usize {
    ffi_guard(0, || {
        unsafe { engine.as_ref() }
            .and_then(|engine| engine.pending_replacement.as_ref())
            .map(|replacement| replacement.delete_count)
            .unwrap_or(0)
    })
}

/// Returns the UTF-8 byte length of the pending replacement text, excluding a
/// trailing NUL.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_pending_replacement_utf8_len(
    engine: *mut TfEngine,
) -> usize {
    ffi_guard(0, || {
        unsafe { engine.as_ref() }
            .and_then(|engine| engine.pending_replacement.as_ref())
            .map(|replacement| replacement.text.len())
            .unwrap_or(0)
    })
}

/// Returns the UTF-8 byte length of the inverse pending replacement text,
/// excluding a trailing NUL.
///
/// This is the text currently represented by the active token before the
/// pending replacement is applied. Hosts can keep it to toggle a manual
/// replacement back without reading text from the foreground app.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_pending_replacement_inverse_utf8_len(
    engine: *mut TfEngine,
) -> usize {
    ffi_guard(0, || {
        unsafe { engine.as_ref() }
            .and_then(|engine| engine.pending_replacement.as_ref())
            .map(|replacement| replacement.inverse_text.len())
            .unwrap_or(0)
    })
}

fn copy_utf8_to_c_buffer(bytes: &[u8], out_utf8: *mut c_char, out_utf8_capacity: usize) {
    if !out_utf8.is_null() && out_utf8_capacity > 0 {
        let copy_len = bytes.len().min(out_utf8_capacity.saturating_sub(1));
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), out_utf8.cast::<u8>(), copy_len);
            *out_utf8.add(copy_len) = 0;
        }
    }
}

/// Copies the inverse pending replacement text as a NUL-terminated UTF-8 string.
///
/// Returns the full required byte length, excluding the trailing NUL. If
/// `out_utf8_capacity` is too small, the written string is truncated but still
/// NUL-terminated when capacity is non-zero. Copying does not clear the pending
/// replacement.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow
/// constructor. `out_utf8` must be null or point to writable memory for
/// `out_utf8_capacity` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_copy_pending_replacement_inverse_utf8(
    engine: *mut TfEngine,
    out_utf8: *mut c_char,
    out_utf8_capacity: usize,
) -> usize {
    ffi_guard(0, || {
        let Some(replacement) =
            (unsafe { engine.as_ref() }).and_then(|engine| engine.pending_replacement.as_ref())
        else {
            copy_utf8_to_c_buffer(&[], out_utf8, out_utf8_capacity);
            return 0;
        };

        let bytes = replacement.inverse_text.as_bytes();
        copy_utf8_to_c_buffer(bytes, out_utf8, out_utf8_capacity);
        bytes.len()
    })
}

/// Takes the pending replacement text as a NUL-terminated UTF-8 string.
///
/// Returns the full required byte length, excluding the trailing NUL. If
/// `out_utf8_capacity` is too small, the written string is truncated but still
/// NUL-terminated when capacity is non-zero. Taking the replacement clears it.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow
/// constructor. `out_utf8` must be null or point to writable memory for
/// `out_utf8_capacity` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_take_pending_replacement_utf8(
    engine: *mut TfEngine,
    out_utf8: *mut c_char,
    out_utf8_capacity: usize,
) -> usize {
    ffi_guard(0, || {
        let Some(engine) = (unsafe { engine.as_mut() }) else {
            copy_utf8_to_c_buffer(&[], out_utf8, out_utf8_capacity);
            return 0;
        };

        let Some(replacement) = engine.pending_replacement.take() else {
            copy_utf8_to_c_buffer(&[], out_utf8, out_utf8_capacity);
            return 0;
        };

        let bytes = replacement.text.as_bytes();
        copy_utf8_to_c_buffer(bytes, out_utf8, out_utf8_capacity);
        bytes.len()
    })
}

/// Observes a single input event and writes the resulting state notification.
///
/// `engine` and `out_observation` must be non-null and valid. `out_observation`
/// is fully overwritten.
///
/// # Safety
///
/// `engine` must be a valid live pointer returned by any Typeflow constructor.
/// `out_observation` must point to writable memory for one `TfObservation`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_observe(
    engine: *mut TfEngine,
    event: TfEvent,
    out_observation: *mut TfObservation,
) {
    ffi_guard_void(|| {
        let Some(engine) = (unsafe { engine.as_mut() }) else {
            return;
        };
        let Some(out) = (unsafe { out_observation.as_mut() }) else {
            return;
        };
        out.write(ObservationAction::None);
        match decode_event(event) {
            Some(input) => {
                let action = {
                    let output = engine.engine.observe(input);
                    let action = output.action.clone();
                    out.write(action.clone());
                    action
                };
                match action {
                    ObservationAction::SwitchFutureLayout(layout) => {
                        engine.set_pending_replacement(layout);
                    }
                    ObservationAction::None | ObservationAction::ResetToken => {
                        engine.clear_pending_replacement();
                    }
                }
            }
            None => {
                engine.clear_pending_replacement();
                out.write(ObservationAction::None);
            }
        }
    });
}

/// Writes the default runtime config into `out_config`.
///
/// # Safety
///
/// `out_config` must be null or point to writable memory for one `TfEngineConfig`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_default_config(out_config: *mut TfEngineConfig) {
    ffi_guard_void(|| {
        if let Some(out) = unsafe { out_config.as_mut() } {
            *out = default_ffi_config();
        }
    });
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        TF_CONTEXT_AUTOMATIC_PROCESSING_DISABLED, TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED,
        TF_CONTEXT_SECURE_INPUT, TF_EVENT_BACKSPACE, TF_EVENT_END_TOKEN, TF_EVENT_LETTER,
        TF_EVENT_LITERAL, TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED,
        TF_HOST_POLICY_MANUAL_SWITCH_DISABLED,
        TF_HOST_POLICY_REASON_AUTOMATIC_PROCESSING_DISABLED_BUNDLE,
        TF_HOST_POLICY_REASON_TERMINAL_BUNDLE, TF_HOST_POLICY_REASON_TERMINAL_SURFACE,
        TF_HOST_POLICY_SECURE_INPUT, TF_HOST_POLICY_TERMINAL_SURFACE, TF_LAYOUT_ENGLISH,
        TF_LAYOUT_SECONDARY, TF_MOD_COMMAND, TF_MOD_CONTROL, TF_MOD_OPTION, TF_MOD_SHIFT,
        TF_OBSERVATION_NONE, TF_OBSERVATION_RESET_TOKEN, TF_OBSERVATION_SWITCH_FUTURE_LAYOUT,
        TfEngineConfig, TfEvent, TfHostInputPolicy, TfHostSurfaceFacts, TfObservation,
        decode_event, default_ffi_config, engine_config_from_ffi,
        typeflow_engine_copy_pending_replacement_inverse_utf8, typeflow_engine_default_config,
        typeflow_engine_force_switch_layout, typeflow_engine_free,
        typeflow_engine_new_embedded_with_config, typeflow_engine_new_from_host_config,
        typeflow_engine_observe, typeflow_engine_pending_replacement_delete_count,
        typeflow_engine_pending_replacement_inverse_utf8_len,
        typeflow_engine_pending_replacement_utf8_len,
        typeflow_engine_take_pending_replacement_utf8,
        typeflow_host_config_auto_disabled_bundle_count, typeflow_host_config_data_directory,
        typeflow_host_config_disabled_bundle_count, typeflow_host_config_engine_config,
        typeflow_host_config_engine_source, typeflow_host_config_free,
        typeflow_host_config_is_automatic_processing_disabled,
        typeflow_host_config_is_bundle_disabled, typeflow_host_config_load_with_environment,
        typeflow_host_config_macos_english_input_source_id,
        typeflow_host_config_macos_secondary_input_source_id, typeflow_host_config_pack_directory,
        typeflow_host_config_resolve_input_policy, typeflow_host_config_secondary_language,
        typeflow_host_config_source_path, typeflow_last_error_message,
    };
    use typeflow_core::InputEvent;

    #[test]
    fn literal_event_decodes_to_literal_input() {
        let input = decode_event(TfEvent {
            tag: TF_EVENT_LITERAL,
            physical: 0,
            modifiers: 0,
            codepoint: '1' as u32,
        });

        assert_eq!(input, Some(InputEvent::Literal('1')));
    }

    #[test]
    fn invalid_literal_codepoint_is_rejected() {
        let input = decode_event(TfEvent {
            tag: TF_EVENT_LITERAL,
            physical: 0,
            modifiers: 0,
            codepoint: 0xD800,
        });

        assert_eq!(input, None);
    }

    #[test]
    fn config_rejects_invalid_numbers() {
        let config = TfEngineConfig {
            min_token_len: 4,
            max_token_len: 128,
            confidence_margin: f32::NAN,
            dict_exact_weight: 5.0,
            dict_prefix_weight: 2.0,
            ngram_only_confidence_margin: 3.0,
            bigram_weight: 1.0,
            trigram_weight: 1.0,
            length_normalize: 1,
            disable_on_internal_caps: 1,
        };

        assert!(engine_config_from_ffi(config).is_none());
    }

    #[test]
    fn constructor_rejects_invalid_config() {
        let config = TfEngineConfig {
            min_token_len: 0,
            max_token_len: 128,
            confidence_margin: 1.0,
            dict_exact_weight: 5.0,
            dict_prefix_weight: 2.0,
            ngram_only_confidence_margin: 3.0,
            bigram_weight: 1.0,
            trigram_weight: 1.0,
            length_normalize: 1,
            disable_on_internal_caps: 1,
        };

        let engine = typeflow_engine_new_embedded_with_config(config);

        assert!(engine.is_null());
    }

    #[test]
    fn config_rejects_max_token_len_above_supported_limit() {
        let mut config = default_ffi_config();
        config.max_token_len = typeflow_core::MAX_CONFIG_TOKEN_LEN + 1;

        assert!(engine_config_from_ffi(config).is_none());

        let engine = typeflow_engine_new_embedded_with_config(config);
        assert!(engine.is_null());
    }

    #[test]
    fn config_rejects_min_token_len_above_max_token_len() {
        let mut config = default_ffi_config();
        config.min_token_len = 9;
        config.max_token_len = 8;

        assert!(engine_config_from_ffi(config).is_none());

        let engine = typeflow_engine_new_embedded_with_config(config);
        assert!(engine.is_null());
    }

    #[test]
    fn observe_with_null_engine_leaves_observation_unchanged() {
        let mut observation = TfObservation {
            tag: TF_OBSERVATION_RESET_TOKEN,
            layout: TF_LAYOUT_SECONDARY,
        };

        unsafe {
            typeflow_engine_observe(
                std::ptr::null_mut(),
                TfEvent {
                    tag: TF_EVENT_LETTER,
                    physical: 0,
                    modifiers: 0,
                    codepoint: 0,
                },
                &mut observation,
            );
        }

        assert_eq!(observation.tag, TF_OBSERVATION_RESET_TOKEN);
        assert_eq!(observation.layout, TF_LAYOUT_SECONDARY);
    }

    #[test]
    fn default_config_accepts_null_output_pointer() {
        unsafe {
            typeflow_engine_default_config(std::ptr::null_mut());
        }
    }

    #[test]
    fn ffi_guard_catches_panic_and_sets_last_error() {
        let old_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let value = super::ffi_guard(7, || -> i32 {
            panic!("ffi boundary test");
        });
        std::panic::set_hook(old_hook);

        assert_eq!(value, 7);
        let error = unsafe { c_string(typeflow_last_error_message()) };
        assert!(error.contains("panic crossed Typeflow FFI boundary: ffi boundary test"));
        super::clear_last_error();
    }

    #[test]
    fn none_observation_clears_previous_payload_metadata() {
        let mut observation = TfObservation {
            tag: TF_OBSERVATION_SWITCH_FUTURE_LAYOUT,
            layout: TF_LAYOUT_SECONDARY,
        };

        observation.write(typeflow_core::ObservationAction::None);

        assert_eq!(observation.tag, TF_OBSERVATION_NONE);
        assert_eq!(observation.layout, TF_LAYOUT_ENGLISH);
    }

    #[test]
    fn observe_returns_state_notifications_only() {
        let engine = typeflow_engine_new_embedded_with_config(default_ffi_config());
        assert!(!engine.is_null());

        let mut observation = empty_observation();
        let mut saw_secondary_switch = false;
        for (index, physical) in [6, 7, 18, 3, 1, 13].into_iter().enumerate() {
            unsafe {
                typeflow_engine_observe(
                    engine,
                    TfEvent {
                        tag: TF_EVENT_LETTER,
                        physical,
                        modifiers: 0,
                        codepoint: 0,
                    },
                    &mut observation,
                );
            }
            saw_secondary_switch |= observation.tag == TF_OBSERVATION_SWITCH_FUTURE_LAYOUT
                && observation.layout == TF_LAYOUT_SECONDARY;
            if saw_secondary_switch {
                assert_eq!(
                    unsafe { typeflow_engine_pending_replacement_delete_count(engine) },
                    index + 1
                );
                assert!(pending_replacement_text(engine).is_some());
                break;
            }
        }

        assert!(saw_secondary_switch);

        unsafe {
            typeflow_engine_observe(engine, end_token(), &mut observation);
        }
        assert_eq!(observation.tag, TF_OBSERVATION_RESET_TOKEN);

        unsafe {
            typeflow_engine_free(engine);
        }
    }

    #[test]
    fn force_switch_changes_future_layout_and_resets_token() {
        let engine = typeflow_engine_new_embedded_with_config(default_ffi_config());
        assert!(!engine.is_null());

        let mut observation = empty_observation();
        for physical in [19, 24, 15, 4] {
            unsafe {
                typeflow_engine_observe(
                    engine,
                    TfEvent {
                        tag: TF_EVENT_LETTER,
                        physical,
                        modifiers: 0,
                        codepoint: 0,
                    },
                    &mut observation,
                );
            }
        }

        unsafe {
            typeflow_engine_force_switch_layout(engine, &mut observation);
        }
        assert_eq!(observation.tag, TF_OBSERVATION_SWITCH_FUTURE_LAYOUT);
        assert_eq!(observation.layout, TF_LAYOUT_SECONDARY);
        assert_eq!(
            unsafe { typeflow_engine_pending_replacement_delete_count(engine) },
            4
        );
        assert_eq!(
            pending_replacement_inverse_text(engine).as_deref(),
            Some("type")
        );
        assert_eq!(pending_replacement_text(engine).as_deref(), Some("ензу"));
        assert_eq!(
            unsafe { typeflow_engine_pending_replacement_delete_count(engine) },
            0
        );

        unsafe {
            typeflow_engine_free(engine);
        }
    }

    #[test]
    fn host_config_loads_resolved_values_and_builds_engine() {
        let dir = temp_dir("host-config");
        let config_path = dir.join("config.toml");
        fs::write(
            &config_path,
            r#"
[engine]
min_token_len = 5

[language]
secondary = " uk "

[packs]
directory = "/config/packs"

[data]
directory = "/config/data"

[apps]
disable_bundle_ids = ["dev.zed.Zed", "com.tinyspeck.slackmacgap"]
disable_auto_bundle_ids = ["com.tinyspeck.slackmacgap", "com.apple.TextEdit"]

[macos]
english_input_source_id = " com.apple.keylayout.ABC "
secondary_input_source_id = " com.apple.keylayout.Ukrainian "
"#,
        )
        .unwrap();

        let path = CString::new(config_path.to_string_lossy().as_bytes()).unwrap();
        let home = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        let data = CString::new("/env/data").unwrap();
        let packs = CString::new("/env/packs").unwrap();
        let config = unsafe {
            typeflow_host_config_load_with_environment(
                path.as_ptr(),
                home.as_ptr(),
                data.as_ptr(),
                packs.as_ptr(),
            )
        };
        assert!(!config.is_null());

        let mut engine_config = default_ffi_config();
        unsafe {
            typeflow_host_config_engine_config(config, &mut engine_config);
        }
        assert_eq!(engine_config.min_token_len, 5);

        assert_eq!(
            unsafe { c_string(typeflow_host_config_source_path(config)) },
            config_path.to_string_lossy()
        );
        assert_eq!(
            unsafe { c_string(typeflow_host_config_secondary_language(config)) },
            "uk"
        );
        assert_eq!(
            unsafe { c_string(typeflow_host_config_pack_directory(config)) },
            "/env/packs"
        );
        assert_eq!(
            unsafe { c_string(typeflow_host_config_data_directory(config)) },
            "/env/data"
        );
        assert_eq!(
            unsafe { c_string(typeflow_host_config_engine_source(config)) },
            "data_dir"
        );
        assert_eq!(
            unsafe { c_string(typeflow_host_config_macos_english_input_source_id(config)) },
            "com.apple.keylayout.ABC"
        );
        assert_eq!(
            unsafe { c_string(typeflow_host_config_macos_secondary_input_source_id(config)) },
            "com.apple.keylayout.Ukrainian"
        );
        assert_eq!(
            unsafe { typeflow_host_config_disabled_bundle_count(config) },
            2
        );
        assert_eq!(
            unsafe { typeflow_host_config_auto_disabled_bundle_count(config) },
            1
        );

        let zed = CString::new("dev.zed.Zed").unwrap();
        let slack = CString::new("com.tinyspeck.slackmacgap").unwrap();
        assert_eq!(
            unsafe { typeflow_host_config_is_bundle_disabled(config, zed.as_ptr()) },
            1
        );
        assert_eq!(
            unsafe { typeflow_host_config_is_automatic_processing_disabled(config, zed.as_ptr()) },
            1
        );
        assert_eq!(
            unsafe { typeflow_host_config_is_bundle_disabled(config, slack.as_ptr()) },
            1
        );
        assert_eq!(
            unsafe {
                typeflow_host_config_is_automatic_processing_disabled(config, slack.as_ptr())
            },
            1
        );
        let textedit = CString::new("com.apple.TextEdit").unwrap();
        assert_eq!(
            unsafe { typeflow_host_config_is_bundle_disabled(config, textedit.as_ptr()) },
            0
        );
        assert_eq!(
            unsafe {
                typeflow_host_config_is_automatic_processing_disabled(config, textedit.as_ptr())
            },
            1
        );
        let mut facts = empty_host_surface_facts();
        facts.bundle_id_utf8 = textedit.as_ptr();
        let mut policy = TfHostInputPolicy {
            flags: 0,
            reason: 0,
        };
        unsafe {
            typeflow_host_config_resolve_input_policy(config, facts, &mut policy);
        }
        assert_eq!(
            policy.reason,
            TF_HOST_POLICY_REASON_AUTOMATIC_PROCESSING_DISABLED_BUNDLE
        );

        // Engine construction fails here because /env/data is intentionally not
        // a language data directory. The constructor decision still lives in Rust.
        assert!(unsafe { typeflow_engine_new_from_host_config(config) }.is_null());
        let error = unsafe { c_string(typeflow_last_error_message()) };
        assert!(error.contains("failed to load language data"), "{error}");

        unsafe {
            typeflow_host_config_free(config);
        }
    }

    #[test]
    fn host_config_defaults_create_embedded_engine() {
        let config = unsafe {
            typeflow_host_config_load_with_environment(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert!(!config.is_null());
        assert_eq!(
            unsafe { c_string(typeflow_host_config_secondary_language(config)) },
            "uk"
        );
        assert_eq!(
            unsafe { c_string(typeflow_host_config_engine_source(config)) },
            "embedded"
        );

        let engine = unsafe { typeflow_engine_new_from_host_config(config) };
        assert!(!engine.is_null());

        unsafe {
            typeflow_engine_free(engine);
            typeflow_host_config_free(config);
        }
    }

    #[test]
    fn host_input_policy_blocks_terminal_bundles_and_surfaces() {
        let config = unsafe {
            typeflow_host_config_load_with_environment(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert!(!config.is_null());

        let bundle = CString::new("com.googlecode.iterm2").unwrap();
        let mut facts = empty_host_surface_facts();
        facts.bundle_id_utf8 = bundle.as_ptr();
        let mut policy = TfHostInputPolicy {
            flags: 0,
            reason: 0,
        };
        unsafe {
            typeflow_host_config_resolve_input_policy(config, facts, &mut policy);
        }
        assert_eq!(policy.reason, TF_HOST_POLICY_REASON_TERMINAL_BUNDLE);
        assert_ne!(
            policy.flags & TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED,
            0
        );
        assert_ne!(policy.flags & TF_HOST_POLICY_MANUAL_SWITCH_DISABLED, 0);
        assert_ne!(policy.flags & TF_HOST_POLICY_TERMINAL_SURFACE, 0);

        let zed = CString::new("dev.zed.Zed").unwrap();
        let terminal_identifier = CString::new("workspace-terminal-panel").unwrap();
        let mut facts = empty_host_surface_facts();
        facts.bundle_id_utf8 = zed.as_ptr();
        facts.focused_element_identifier_utf8 = terminal_identifier.as_ptr();
        unsafe {
            typeflow_host_config_resolve_input_policy(config, facts, &mut policy);
        }
        assert_eq!(policy.reason, TF_HOST_POLICY_REASON_TERMINAL_SURFACE);
        assert_ne!(
            policy.flags & TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED,
            0
        );
        assert_ne!(policy.flags & TF_HOST_POLICY_MANUAL_SWITCH_DISABLED, 0);
        assert_ne!(policy.flags & TF_HOST_POLICY_TERMINAL_SURFACE, 0);

        let terminal_context = CString::new("AXGroup workspace-terminal-panel").unwrap();
        let mut facts = empty_host_surface_facts();
        facts.bundle_id_utf8 = zed.as_ptr();
        facts.focused_element_context_utf8 = terminal_context.as_ptr();
        unsafe {
            typeflow_host_config_resolve_input_policy(config, facts, &mut policy);
        }
        assert_eq!(policy.reason, TF_HOST_POLICY_REASON_TERMINAL_SURFACE);
        assert_ne!(
            policy.flags & TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED,
            0
        );
        assert_ne!(policy.flags & TF_HOST_POLICY_MANUAL_SWITCH_DISABLED, 0);
        assert_ne!(policy.flags & TF_HOST_POLICY_TERMINAL_SURFACE, 0);

        unsafe {
            typeflow_host_config_free(config);
        }
    }

    #[test]
    fn host_input_policy_marks_secure_input() {
        let config = unsafe {
            typeflow_host_config_load_with_environment(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert!(!config.is_null());

        let mut facts = empty_host_surface_facts();
        facts.secure_input = 1;
        let mut policy = TfHostInputPolicy {
            flags: 0,
            reason: 0,
        };
        unsafe {
            typeflow_host_config_resolve_input_policy(config, facts, &mut policy);
        }
        assert_ne!(policy.flags & TF_HOST_POLICY_SECURE_INPUT, 0);
        assert_ne!(
            policy.flags & TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED,
            0
        );
        assert_ne!(policy.flags & TF_HOST_POLICY_MANUAL_SWITCH_DISABLED, 0);

        unsafe {
            typeflow_host_config_free(config);
        }
    }

    #[test]
    fn invalid_host_config_sets_last_error() {
        let dir = temp_dir("invalid-host-config");
        let config_path = dir.join("config.toml");
        fs::write(
            &config_path,
            r#"
[apps]
disable_bundle_ids = [
    "dev.zed.Zed",
"#,
        )
        .unwrap();

        let path = CString::new(config_path.to_string_lossy().as_bytes()).unwrap();
        let config = unsafe {
            typeflow_host_config_load_with_environment(
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert!(config.is_null());

        let error = unsafe { c_string(typeflow_last_error_message()) };
        assert!(error.contains("parse config"), "{error}");
        assert!(error.contains("unclosed array"), "{error}");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn public_header_constants_match_rust_abi() {
        let header = include_str!("../include/typeflow.h");
        for (name, value) in [
            ("TF_EVENT_LETTER", TF_EVENT_LETTER as usize),
            ("TF_EVENT_BACKSPACE", TF_EVENT_BACKSPACE as usize),
            ("TF_EVENT_END_TOKEN", TF_EVENT_END_TOKEN as usize),
            ("TF_EVENT_LITERAL", TF_EVENT_LITERAL as usize),
            ("TF_MOD_SHIFT", TF_MOD_SHIFT as usize),
            ("TF_MOD_CONTROL", TF_MOD_CONTROL as usize),
            ("TF_MOD_OPTION", TF_MOD_OPTION as usize),
            ("TF_MOD_COMMAND", TF_MOD_COMMAND as usize),
            ("TF_CONTEXT_SECURE_INPUT", TF_CONTEXT_SECURE_INPUT as usize),
            (
                "TF_CONTEXT_AUTOMATIC_PROCESSING_DISABLED",
                TF_CONTEXT_AUTOMATIC_PROCESSING_DISABLED as usize,
            ),
            (
                "TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED",
                TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED as usize,
            ),
            (
                "TF_HOST_POLICY_SECURE_INPUT",
                TF_HOST_POLICY_SECURE_INPUT as usize,
            ),
            (
                "TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED",
                TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED as usize,
            ),
            (
                "TF_HOST_POLICY_MANUAL_SWITCH_DISABLED",
                TF_HOST_POLICY_MANUAL_SWITCH_DISABLED as usize,
            ),
            (
                "TF_HOST_POLICY_TERMINAL_SURFACE",
                TF_HOST_POLICY_TERMINAL_SURFACE as usize,
            ),
            ("TF_OBSERVATION_NONE", TF_OBSERVATION_NONE as usize),
            (
                "TF_OBSERVATION_SWITCH_FUTURE_LAYOUT",
                TF_OBSERVATION_SWITCH_FUTURE_LAYOUT as usize,
            ),
            (
                "TF_OBSERVATION_RESET_TOKEN",
                TF_OBSERVATION_RESET_TOKEN as usize,
            ),
            ("TF_LAYOUT_ENGLISH", TF_LAYOUT_ENGLISH as usize),
            ("TF_LAYOUT_SECONDARY", TF_LAYOUT_SECONDARY as usize),
        ] {
            assert_eq!(header_define(header, name), value, "{name}");
        }
    }

    fn empty_observation() -> TfObservation {
        TfObservation {
            tag: TF_OBSERVATION_NONE,
            layout: TF_LAYOUT_ENGLISH,
        }
    }

    fn end_token() -> TfEvent {
        TfEvent {
            tag: TF_EVENT_END_TOKEN,
            physical: 0,
            modifiers: 0,
            codepoint: 0,
        }
    }

    fn pending_replacement_text(engine: *mut super::TfEngine) -> Option<String> {
        let len = unsafe { typeflow_engine_pending_replacement_utf8_len(engine) };
        if len == 0 {
            return None;
        }

        let mut buffer = vec![0i8; len + 1];
        let written = unsafe {
            typeflow_engine_take_pending_replacement_utf8(engine, buffer.as_mut_ptr(), buffer.len())
        };
        assert_eq!(written, len);
        Some(
            unsafe { CStr::from_ptr(buffer.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn pending_replacement_inverse_text(engine: *mut super::TfEngine) -> Option<String> {
        let len = unsafe { typeflow_engine_pending_replacement_inverse_utf8_len(engine) };
        if len == 0 {
            return None;
        }

        let mut buffer = vec![0i8; len + 1];
        let written = unsafe {
            typeflow_engine_copy_pending_replacement_inverse_utf8(
                engine,
                buffer.as_mut_ptr(),
                buffer.len(),
            )
        };
        assert_eq!(written, len);
        Some(
            unsafe { CStr::from_ptr(buffer.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    unsafe fn c_string(pointer: *const std::os::raw::c_char) -> String {
        assert!(!pointer.is_null());
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }

    fn empty_host_surface_facts() -> TfHostSurfaceFacts {
        TfHostSurfaceFacts {
            secure_input: 0,
            bundle_id_utf8: std::ptr::null(),
            application_name_utf8: std::ptr::null(),
            input_client_class_utf8: std::ptr::null(),
            focused_element_role_utf8: std::ptr::null(),
            focused_element_subrole_utf8: std::ptr::null(),
            focused_element_role_description_utf8: std::ptr::null(),
            focused_element_identifier_utf8: std::ptr::null(),
            focused_element_description_utf8: std::ptr::null(),
            focused_element_context_utf8: std::ptr::null(),
            focused_window_title_utf8: std::ptr::null(),
        }
    }

    fn header_define(header: &str, name: &str) -> usize {
        let prefix = format!("#define {name}");
        let line = header
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("missing header define {name}"));
        let value = line[prefix.len()..]
            .trim()
            .trim_end_matches('u')
            .trim_end_matches('U');
        value
            .strip_prefix("0x")
            .map(|hex| usize::from_str_radix(hex, 16))
            .unwrap_or_else(|| value.parse())
            .unwrap_or_else(|error| panic!("invalid header define {name}={value}: {error}"))
    }

    fn temp_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "typeflow-ffi-{name}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
