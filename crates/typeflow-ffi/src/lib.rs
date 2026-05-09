use std::ffi::CStr;
use std::os::raw::c_char;
use std::path::PathBuf;

use typeflow_core::data::LanguageBundle;
use typeflow_core::{
    Action, Engine, EngineConfig, InputEvent, Layout, LetterEvent, MAX_CONFIG_TOKEN_LEN,
    PhysicalKey,
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
pub const TF_CONTEXT_APP_EXCLUDED: u32 = 0x02;

pub const TF_ACTION_KEEP: u8 = 0;
pub const TF_ACTION_COMMIT: u8 = 1;
pub const TF_ACTION_REPLACE: u8 = 2;
pub const TF_ACTION_RESET: u8 = 3;

pub const TF_LAYOUT_ENGLISH: u8 = 0;
pub const TF_LAYOUT_SECONDARY: u8 = 1;

pub const TF_REPLACE_BUF_LEN: usize = 4096;
const TF_MAX_TOKEN_LEN: usize = TF_REPLACE_BUF_LEN / 4;
const _: () = assert!(TF_MAX_TOKEN_LEN == MAX_CONFIG_TOKEN_LEN);

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
pub struct TfAction {
    pub tag: u8,
    pub commit_codepoint: u32,
    pub replace_old_len: usize,
    pub replace_text_len: usize,
    pub replace_layout: u8,
    pub replace_text: [u8; TF_REPLACE_BUF_LEN],
}

impl TfAction {
    fn write(&mut self, action: Action) {
        self.tag = TF_ACTION_KEEP;
        self.commit_codepoint = 0;
        self.replace_old_len = 0;
        self.replace_text_len = 0;
        self.replace_layout = TF_LAYOUT_ENGLISH;

        match action {
            Action::Keep => {}
            Action::Commit(character) => {
                self.tag = TF_ACTION_COMMIT;
                self.commit_codepoint = character as u32;
            }
            Action::ReplaceToken {
                old_len,
                replacement,
                layout,
            } => {
                let bytes = replacement.as_bytes();
                if bytes.len() > TF_REPLACE_BUF_LEN {
                    self.tag = TF_ACTION_RESET;
                    return;
                }
                self.tag = TF_ACTION_REPLACE;
                self.replace_old_len = old_len;
                self.replace_text_len = bytes.len();
                self.replace_layout = layout_to_u8(layout);
                self.replace_text[..bytes.len()].copy_from_slice(bytes);
                // The ABI is length-delimited; this sentinel is only for
                // defensive debugging by C callers that accidentally print the
                // buffer as a string.
                if bytes.len() < TF_REPLACE_BUF_LEN {
                    self.replace_text[bytes.len()] = 0;
                }
            }
            Action::ResetToken => {
                self.tag = TF_ACTION_RESET;
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
    let config = EngineConfig::default();
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

fn new_engine(bundle: LanguageBundle, config: TfEngineConfig) -> *mut Engine {
    let Some(config) = engine_config_from_ffi(config) else {
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(Engine::new(config, bundle)))
}

fn host_context_from_flags(flags: u32) -> typeflow_core::HostContext {
    typeflow_core::HostContext {
        secure_input: flags & TF_CONTEXT_SECURE_INPUT != 0,
        app_excluded: flags & TF_CONTEXT_APP_EXCLUDED != 0,
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
/// This is the normal constructor for release builds and the future macOS input method.
/// Returns null if the embedded bundle fails to deserialize.
#[unsafe(no_mangle)]
pub extern "C" fn typeflow_engine_new_embedded() -> *mut Engine {
    typeflow_engine_new_embedded_with_config(default_ffi_config())
}

/// Allocates a new embedded engine with explicit runtime tuning.
///
/// Returns null if the embedded bundle fails to deserialize or the config contains
/// invalid numeric values.
#[unsafe(no_mangle)]
pub extern "C" fn typeflow_engine_new_embedded_with_config(config: TfEngineConfig) -> *mut Engine {
    let Ok(bundle) = LanguageBundle::embedded() else {
        return std::ptr::null_mut();
    };
    new_engine(bundle, config)
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
) -> *mut Engine {
    unsafe { typeflow_engine_new_from_data_dir_with_config(data_dir_utf8, default_ffi_config()) }
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
) -> *mut Engine {
    let Some(path) = (unsafe { c_path(data_dir_utf8) }) else {
        return std::ptr::null_mut();
    };
    let Ok(bundle) = LanguageBundle::from_data_dir(&path) else {
        return std::ptr::null_mut();
    };
    new_engine(bundle, config)
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
) -> *mut Engine {
    unsafe { typeflow_engine_new_from_pack_dir_with_config(pack_dir_utf8, default_ffi_config()) }
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
) -> *mut Engine {
    let Some(path) = (unsafe { c_path(pack_dir_utf8) }) else {
        return std::ptr::null_mut();
    };
    let Ok(bundle) = LanguageBundle::from_secondary_pack_dir(&path) else {
        return std::ptr::null_mut();
    };
    new_engine(bundle, config)
}

unsafe fn c_path(path_utf8: *const c_char) -> Option<PathBuf> {
    if path_utf8.is_null() {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(path_utf8) };
    let path_str = cstr.to_str().ok()?;
    Some(PathBuf::from(path_str))
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
pub unsafe extern "C" fn typeflow_engine_free(engine: *mut Engine) {
    if !engine.is_null() {
        unsafe {
            drop(Box::from_raw(engine));
        }
    }
}

/// Clears the current token buffer without changing the active layout.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_reset_token(engine: *mut Engine) {
    if let Some(engine) = unsafe { engine.as_mut() } {
        engine.reset_token();
    }
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
pub unsafe extern "C" fn typeflow_engine_reset_layout(engine: *mut Engine, layout: u8) {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return;
    };
    let Some(layout) = layout_from_u8(layout) else {
        return;
    };
    engine.reset_layout(layout);
}

/// Sets host-level bypass context.
///
/// `TF_CONTEXT_SECURE_INPUT` means a password/secure field is active.
/// `TF_CONTEXT_APP_EXCLUDED` means the foreground app is user-excluded.
/// While either is set, the engine returns Keep/Bypass and clears the token.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by a Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_set_host_context(engine: *mut Engine, flags: u32) {
    if let Some(engine) = unsafe { engine.as_mut() } {
        engine.set_host_context(host_context_from_flags(flags));
    }
}

/// Forces the current token to the opposite side of the active language pair.
///
/// # Safety
///
/// `engine` must be a valid live pointer returned by `typeflow_engine_new_embedded` or
/// any other Typeflow constructor. `out_action` must point to writable memory for one `TfAction`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_force_switch_token(
    engine: *mut Engine,
    out_action: *mut TfAction,
) {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return;
    };
    let Some(out) = (unsafe { out_action.as_mut() }) else {
        return;
    };

    let output = engine.force_switch_token();
    out.write(output.action);
}

/// Returns the engine's current inferred layout.
///
/// # Safety
///
/// `engine` must be null or a valid live pointer returned by any Typeflow constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_current_layout(engine: *mut Engine) -> u8 {
    match unsafe { engine.as_ref() } {
        Some(engine) => layout_to_u8(engine.current_layout()),
        None => TF_LAYOUT_ENGLISH,
    }
}

/// Processes a single input event and writes the resulting action into `out_action`.
///
/// `engine` and `out_action` must be non-null and valid. `out_action` is fully overwritten.
///
/// # Safety
///
/// `engine` must be a valid live pointer returned by any Typeflow constructor.
/// `out_action` must point to writable memory for one `TfAction`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_process(
    engine: *mut Engine,
    event: TfEvent,
    out_action: *mut TfAction,
) {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return;
    };
    let Some(out) = (unsafe { out_action.as_mut() }) else {
        return;
    };
    match decode_event(event) {
        Some(input) => {
            let action = engine.process_action(input);
            out.write(action);
        }
        None => out.write(Action::Keep),
    }
}

/// Writes the default runtime config into `out_config`.
///
/// # Safety
///
/// `out_config` must be null or point to writable memory for one `TfEngineConfig`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typeflow_engine_default_config(out_config: *mut TfEngineConfig) {
    if let Some(out) = unsafe { out_config.as_mut() } {
        *out = default_ffi_config();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TF_ACTION_COMMIT, TF_ACTION_KEEP, TF_EVENT_LETTER, TF_EVENT_LITERAL, TF_REPLACE_BUF_LEN,
        TfAction, TfEngineConfig, TfEvent, decode_event, default_ffi_config,
        engine_config_from_ffi, typeflow_engine_default_config,
        typeflow_engine_new_embedded_with_config, typeflow_engine_process,
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
    fn config_rejects_max_token_len_that_can_overflow_replace_buffer() {
        let mut config = default_ffi_config();
        config.max_token_len = TF_REPLACE_BUF_LEN;

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
    fn process_with_null_engine_leaves_action_unchanged() {
        let mut action = TfAction {
            tag: TF_ACTION_COMMIT,
            commit_codepoint: 'x' as u32,
            replace_old_len: 9,
            replace_text_len: 9,
            replace_layout: 1,
            replace_text: [1; super::TF_REPLACE_BUF_LEN],
        };

        unsafe {
            typeflow_engine_process(
                std::ptr::null_mut(),
                TfEvent {
                    tag: TF_EVENT_LETTER,
                    physical: 0,
                    modifiers: 0,
                    codepoint: 0,
                },
                &mut action,
            );
        }

        assert_eq!(action.tag, TF_ACTION_COMMIT);
        assert_eq!(action.commit_codepoint, 'x' as u32);
    }

    #[test]
    fn default_config_accepts_null_output_pointer() {
        unsafe {
            typeflow_engine_default_config(std::ptr::null_mut());
        }
    }

    #[test]
    fn keep_action_clears_previous_payload_metadata() {
        let mut action = TfAction {
            tag: TF_ACTION_COMMIT,
            commit_codepoint: 'x' as u32,
            replace_old_len: 9,
            replace_text_len: 9,
            replace_layout: 1,
            replace_text: [1; super::TF_REPLACE_BUF_LEN],
        };

        action.write(typeflow_core::Action::Keep);

        assert_eq!(action.tag, TF_ACTION_KEEP);
        assert_eq!(action.commit_codepoint, 0);
        assert_eq!(action.replace_old_len, 0);
        assert_eq!(action.replace_text_len, 0);
    }
}
