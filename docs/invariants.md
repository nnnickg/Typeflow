# Core Invariants

This is the contract the Rust core expects every host to respect. The macOS
IMK layer should be written against this file, not against CLI behavior by
accident.

## Language Pair

- The engine scores exactly two sides: `Layout::English` and
  `Layout::Secondary`.
- English is the fixed primary side.
- `Secondary` is language-pack backed. The release binary embeds Ukrainian
  (`uk`); any other secondary language is loaded from an installed pack.
- `Layout::Secondary` is a role, not a specific language. Host code must not
  bake in assumptions about the active secondary language.

## Physical Keys

- `PhysicalKey` is the ABI-facing key-position model. Its numeric indices are
  stable API.
- Current indices are `A..Z = 0..25`, `Grave = 26`, `LBracket = 27`,
  `RBracket = 28`, `Semicolon = 29`, `Quote = 30`, `Comma = 31`,
  `Period = 32`, `Backslash = 33`.
- A keycode-level host must send physical key positions, not rendered
  characters.
- A text-driven caller may use `Engine::input_event_from_char`; that path
  reverse-maps through the loaded keyboard maps and falls back to literals.
- Shift is part of `LetterEvent`, not a separate token. Shifted and unshifted
  letters share the same physical key with different `shift`.
- Ctrl, Option, Command, or Function modified events are host bypass events.
  The engine must not consume or transform shortcuts.

## Token State

- The token buffer contains only `LetterEvent` values.
- `candidates.english` and `candidates.secondary` are renderings of the same
  token under the two active keyboard maps.
- Candidate character counts must match the token length.
- `reset_token()` clears token, candidates, and score cache without changing
  the active layout.
- `reset_layout(layout)` changes active layout and clears token, candidates,
  and score cache.
- `EndToken` clears token state and returns `Action::ResetToken`.
- `Literal(char)` clears token state and returns `Action::Commit(char)`.
- `Backspace` removes one `LetterEvent` from token state, reconciles the
  inferred layout for the shortened token, and returns `Action::Keep`.
- Backspace on an empty token is a no-op.
- Hosts must call `EndToken` or `reset_token()` on focus loss, app switch,
  committed whitespace, or any other boundary that makes the previous letters
  no longer replaceable as one contiguous token.

## Action Protocol

- `Action::Commit(c)` means insert exactly one Unicode scalar into the host
  buffer.
- `Action::ReplaceToken { old_len, replacement, layout }` means delete exactly
  `old_len` already-committed trailing characters, then insert `replacement`.
- During automatic switching, `old_len` intentionally excludes the current
  just-pressed key. The replacement includes the whole token including that key.
- During `force_switch_token()`, `old_len` is the full token length because all
  letters in the token have already been committed by earlier actions.
- `Action::Keep` means the host must not insert, delete, or reset visible text.
- `Action::ResetToken` means host-side token bookkeeping should reset, but no
  visible text should be edited by the core action itself.
- The host must apply actions in order. Reordering, coalescing, or dropping a
  `ReplaceToken` will desynchronize the host buffer and engine token state.

## Switching Rules

- Tokens shorter than `min_token_len` never switch.
- Tokens with internal Shift-modified letters bypass switching when
  `disable_on_internal_caps` is enabled.
- Fully shifted acronym-like tokens bypass switching.
- A candidate can win via normal `confidence_margin` only when it has
  dictionary evidence: exact match or prefix evidence.
- A candidate without dictionary evidence must clear
  `ngram_only_confidence_margin`.
- When a token shrinks through backspace, the engine re-scores the shortened
  token. If no layout now wins, it restores the token's start layout.
- Once a token has switched during normal typing, the active layout remains the
  winning layout until another decision, backspace reconciliation, token reset,
  or explicit layout reset changes it.
- `force_switch_token()` bypasses scoring and switches to the opposite layout
  for the current token only.

## Host Context

- `HostContext.secure_input` and `HostContext.app_excluded` are hard bypass
  flags.
- While either flag is true, the engine clears token state and returns
  `Decision::Bypass` with `Action::Keep`.
- The host is responsible for setting these flags before sending letter events
  for secure fields or excluded apps.
- Clearing host context does not restore a previous token. The next letter
  starts a fresh token.

## FFI

- Any constructor may return null. The host must treat null as initialization
  failure and stop using that engine handle.
- Pointers returned by constructors must be freed exactly once with
  `typeflow_engine_free`.
- Passing null to null-tolerant functions is a no-op or English fallback as
  documented in the Rust FFI comments.
- `typeflow_engine_process` requires a valid engine pointer and writable
  `TfAction` pointer. Invalid events decode to `Action::Keep`.
- `TfAction.replace_text` is an inline UTF-8 byte buffer. The host must copy
  exactly `replace_text_len` bytes and decode them as UTF-8.
- `TF_REPLACE_BUF_LEN` bounds replacement payloads. If a replacement ever
  exceeds that buffer, the FFI action writer must fail closed rather than
  exposing partial text.

## Data And Packs

- Embedded data must deserialize into `CompiledLanguageData` whose
  `language_tag` matches the expected pack id.
- Pack manifests must stay within the pack directory. Path traversal is
  invalid.
- Pack ids are stable user-facing identifiers. `en` is reserved and cannot be a
  secondary pack id.
- Pack format compatibility is governed by `PACK_FORMAT_VERSION`.
- `LanguageBundle::embedded()` means embedded English plus embedded Ukrainian.
- `LanguageBundle::from_secondary_pack_dir(path)` means embedded English plus
  exactly one external secondary pack.

## Calibration Boundaries

Calibration may change:

- `EngineConfig` defaults.
- dictionary and n-gram weights.
- curated eval cases.
- corpus generation/filtering.
- pack contents.

Calibration must not change:

- `Action` semantics.
- `PhysicalKey` numeric indices.
- token/candidate length invariants.
- FFI ownership rules.
- the meaning of `Layout::English` versus `Layout::Secondary`.
