# Core Invariants

This is the Rust core and host contract. The macOS observer agent should be
written against this file, not against CLI convenience behavior.

## Language Pair

- The engine scores exactly two sides: `Layout::English` and
  `Layout::Secondary`.
- English is fixed. `Secondary` is language-pack backed; release builds embed
  Ukrainian (`uk`) and can load other installed packs.
- `Layout::Secondary` is a role, not a hardcoded language. Host code must not
  assume Ukrainian-specific text.

## Physical Keys

- `PhysicalKey` is the ABI-facing key-position model. Its numeric indices are
  stable API.
- Current indices are `A..Z = 0..25`, `Grave = 26`, `LBracket = 27`,
  `RBracket = 28`, `Semicolon = 29`, `Quote = 30`, `Comma = 31`,
  `Period = 32`, `Backslash = 33`.
- A keycode-level host sends physical key positions, not rendered characters,
  for normal letters.
- A text-driven caller may use `Engine::input_event_from_char`; that path
  reverse-maps through the loaded keyboard maps and falls back to literals.
- Ctrl, Command, and Option-modified key events are host bypass events.
- Standalone Option may call `force_switch_layout()` as a host command. It is
  not encoded as a `TcEvent`.

## Pass-Through Contract

- TypeClaw is an observer, not an input compositor.
- Normal printable keyDown events pass through to the app through a listen-only
  event tap. TypeClaw must not become the active text compositor.
- The engine never returns text to render or commit during normal typing.
- The host must not call host composition or overlay APIs for normal observed
  letters.
- TypeClaw may update internal token state and inferred future layout on every
  key. It may perform one host token replacement when a switch decision is made.
- Manual Option switching changes future layout state and resets the observed
  token. If a token is active, the macOS host may replace that token once.

## Token State

- The token buffer contains only `LetterEvent` values.
- `candidates.english` and `candidates.secondary` are renderings of the same
  observed token under the two active keyboard maps.
- Candidate character counts must match token length.
- `reset_token()` clears token, candidates, n-grams, and score cache without
  changing the active layout.
- `reset_layout(layout)` changes active layout and clears token state.
- `EndToken`, `Literal(char)`, `HostBypass`, focus loss, app switch, secure
  input, and disabled surfaces reset the observed token.
- English punctuation-position `LetterEvent`s listed by the active secondary
  pack's `punctuation_letter_keys` stay eligible as token letters. They reset
  the observed token only when appending the key leaves the secondary candidate
  without dictionary prefix or exact-word evidence.
- `Backspace` removes one `LetterEvent` from token state and reconciles the
  inferred layout for the shortened token.
- Backspace on an empty token is a no-op for engine state.
- Once a letter-event run exceeds `max_token_len`, the engine resets token
  state and ignores scoring until the next boundary.

## Observation Protocol

- `Engine::observe(InputEvent) -> ObservationOutput` is the hot path.
- `ObservationAction::None` means no host-visible state notification.
- `ObservationAction::SwitchFutureLayout(layout)` means the engine's inferred
  future layout changed.
- `ObservationAction::ResetToken` means the active observed token ended or was
  discarded.
- `SwitchFutureLayout` authorizes a boundary-sized host side effect: replace the
  currently tracked token with the Rust-rendered candidate for `layout`, then
  select the configured real keyboard input source for future keys.
- `None` and `ResetToken` do not authorize document mutation.

## Switching Rules

- Tokens shorter than `min_token_len` never switch.
- Tokens with internal Shift-modified letters bypass switching when
  `disable_on_internal_caps` is enabled.
- Fully shifted acronym-like tokens bypass switching.
- A candidate can win via `confidence_margin` only when it has dictionary
  evidence: exact match or prefix evidence.
- A candidate without dictionary evidence must clear
  `ngram_only_confidence_margin`.
- When a token shrinks through backspace, the engine re-scores the shortened
  token. If no layout now wins, it restores the token's start layout.
- `force_switch_layout()` bypasses scoring, switches to the opposite layout,
  clears the observed token, and returns `SwitchFutureLayout`.

## Host Context

- `HostContext.secure_input` and
  `HostContext.automatic_processing_disabled` are full bypass flags for normal
  key observation.
- While either full bypass flag is true, the engine clears token state and
  returns `Decision::Bypass`.
- `HostContext.automatic_switching_disabled` disables automatic layout
  decisions, but the engine may still observe token candidates in the current
  layout. This backs `apps.disable_auto_bundle_ids`.
- Rust owns app policy from `HostSurfaceFacts`, `apps.disable_bundle_ids`, and
  `apps.disable_auto_bundle_ids`; Swift supplies facts.
- Secure input, terminal-like surfaces, and fully disabled apps bypass
  everything.
- Unavailable config or engine data is a startup failure. The macOS agent must
  report it and terminate instead of running with a nil engine.
- macOS must not do AX discovery synchronously inside key handling. It uses the
  last cached policy; stale or unknown policy defaults to bypass/no replacement
  until the asynchronous refresh completes.

## FFI

- Any constructor may return null. The host must treat null as initialization
  failure and stop using that engine handle.
- Pointers returned by constructors must be freed exactly once with
  `typeclaw_engine_free`.
- Passing null to null-tolerant functions is a no-op or English fallback as
  documented in the Rust FFI comments.
- `typeclaw_engine_observe` requires a valid engine pointer and writable
  `TcObservation` pointer. Invalid events write `ObservationAction::None`.
- `TcObservation` contains only `tag` and `layout`; replacement text is not part
  of the observation payload.
- `typeclaw_engine_observe` and `typeclaw_engine_force_switch_layout` must
  capture a pending replacement snapshot before any token reset that follows a
  `SwitchFutureLayout` action.
- Hosts that need replacement text must consume the pending snapshot with
  `typeclaw_engine_pending_replacement_delete_count`,
  `typeclaw_engine_pending_replacement_utf8_len`, and
  `typeclaw_engine_take_pending_replacement_utf8`. Hosts that support manual
  replacement toggles may also read the inverse snapshot with
  `typeclaw_engine_pending_replacement_inverse_utf8_len` and
  `typeclaw_engine_copy_pending_replacement_inverse_utf8` before taking the
  replacement. Reset, host-context changes, invalid events, and non-switch
  observations clear the snapshot.

## Data And Packs

- Embedded data must deserialize into `CompiledLanguageData` whose
  `language_tag` matches the expected pack id.
- Pack manifests must stay within the pack directory. Path traversal is invalid.
- Pack ids are stable user-facing identifiers. `en` and `uk` are reserved and
  cannot be external secondary pack ids.
- Secondary packs own their `punctuation_letter_keys`; the engine must not
  assume Ukrainian-specific punctuation-key behavior.
- Pack format compatibility is governed by `PACK_FORMAT_VERSION`.
- `docs/artifact-format.md` defines what requires a format-version bump.

## Calibration Boundaries

Calibration may change:

- `EngineConfig` defaults.
- dictionary and n-gram weights.
- curated eval cases.
- corpus generation/filtering.
- pack contents.

Calibration must not change:

- `ObservationAction` semantics.
- `PhysicalKey` numeric indices.
- token/candidate length invariants.
- FFI ownership rules.
- the meaning of `Layout::English` versus `Layout::Secondary`.
