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
- Ctrl, Command, and Option-modified key events are host bypass events. The
  engine must not consume or transform shortcuts.
- A host may bind standalone Option press/release to `force_switch_token()` for
  manual conversion. That modifier transition is host behavior; it is not a
  letter event and is not encoded as a `TfEvent`.

## Token State

- The token buffer contains only `LetterEvent` values.
- `candidates.english` and `candidates.secondary` are renderings of the same
  token under the two active keyboard maps.
- Candidate character counts must match the token length.
- `reset_token()` clears token, candidates, and score cache without changing
  the active layout.
- `reset_layout(layout)` changes active layout and clears token, candidates,
  and score cache.
- `EndToken` commits the active rendered composition once and clears token
  state. If no token is active, it bypasses to the host.
- `Literal(char)` commits the active rendered composition plus the literal
  once, then clears token state. If no token is active, it bypasses to the host.
- Punctuation-looking physical keys that are secondary-layout letters remain
  `Letter` events. If the active layout is English and the current token has
  already resolved as English, English punctuation on those keys terminates the
  token and commits the punctuation character.
- `Backspace` removes one `LetterEvent` from token state, reconciles the
  inferred layout for the shortened token, and returns a render or clear
  composition action.
- Backspace on an empty token bypasses to the host.
- Once a letter-only run exceeds `max_token_len`, the engine commits the
  buffered composition plus the current key once, then bypasses scoring until
  the next token boundary.
- Hosts must call `EndToken` or `reset_token()` on focus loss, app switch,
  committed whitespace, or any other boundary that makes the previous letters
  no longer belongs to the active composition.
- Hosts must reset token state when the focused text client changes or when an
  out-of-band edit invalidates the active composition.

## Composition Protocol

- `CompositionAction::Render { text, layout }` means redraw the active
  Typeflow-owned composition. The host must consume the key event and must not
  let the app insert raw text for that event.
- `CompositionAction::Commit { text, consume_event }` means insert `text` once
  with host commit semantics. If `consume_event` is false, the host may pass the
  original boundary event through after committing the text.
- `CompositionAction::Clear { consume_event }` means clear active rendered
  composition without committing text.
- `CompositionAction::Bypass` means Typeflow is not handling the event. The host
  should let the app process it normally.
- Normal conversion is render-only while a token is active. The document is not
  edited per key and no committed trailing text is replaced.
- `force_switch_token()` changes the internal layout and returns `Render`; it
  does not mutate committed document text.

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

- `HostContext.secure_input` and
  `HostContext.automatic_processing_disabled` are full bypass flags for normal
  key processing.
- While either full bypass flag is true, the engine clears token state and
  returns `Decision::Bypass` with `CompositionAction::Bypass`.
- `HostContext.automatic_switching_disabled` disables automatic layout
  decisions, but the host still uses Typeflow-owned composition in the current
  layout. This is the mode used for apps in `apps.disable_auto_bundle_ids`:
  standalone Option can switch the current layout, and subsequent keys use that
  layout without automatic scoring.
- The host is responsible for setting these flags before sending letter events.
  Secure-input detection is a host signal. App disable policy and
  terminal-surface policy are evaluated by Rust from `HostSurfaceFacts`,
  `apps.disable_bundle_ids`, and `apps.disable_auto_bundle_ids`; the macOS host
  supplies facts, not decisions.
- Secure input, terminal-like surfaces, and fully disabled apps bypass
  everything.
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
  `TfComposition` pointer. Invalid events decode to
  `CompositionAction::Bypass`.
- `TfComposition.text` is an inline UTF-8 byte buffer. The host must copy
  exactly `text_len` bytes and decode them as UTF-8.
- `TF_COMPOSITION_TEXT_BUF_LEN` bounds render and commit payloads. If a payload
  exceeds that buffer, the FFI writer fails closed with a clear composition
  action rather than exposing partial text.

## Data And Packs

- Embedded data must deserialize into `CompiledLanguageData` whose
  `language_tag` matches the expected pack id.
- Pack manifests must stay within the pack directory. Path traversal is
  invalid.
- Pack ids are stable user-facing identifiers. `en` is reserved and cannot be a
  secondary pack id.
- Pack format compatibility is governed by `PACK_FORMAT_VERSION`.
- `docs/artifact-format.md` defines what requires a format-version bump.
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

- `CompositionAction` semantics.
- `PhysicalKey` numeric indices.
- token/candidate length invariants.
- FFI ownership rules.
- the meaning of `Layout::English` versus `Layout::Secondary`.
