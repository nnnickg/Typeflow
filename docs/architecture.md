# Architecture

## High-level shape

```text
                   ┌────────────────────────────────────┐
                   │  macOS InputMethodKit bundle       │
                   │  (Swift/AppKit IMKInputController) │
                   └─────────────┬──────────────────────┘
                                 │ raw keyDown / flagsChanged
                                 ▼
┌────────────────────┐   ┌───────────────────────────────┐   ┌────────────────────┐
│  typeflow-cli      │──▶│  typeflow-core                │◀──│  typeflow-ffi      │
│  type / stream     │   │   - PhysicalKey, InputEvent   │   │  C ABI for IMK     │
│  repl / predict    │   │   - Engine.process()          │   │  (Swift IMK host) │
│  pack / config     │   │   - LanguageBundle scoring    │   │                    │
└─────────┬──────────┘   └───────────┬───────────────────┘   └──────────┬─────────┘
          │                          │                                   │
          └──────────▶ typeflow-host-config ◀────────────────────────────┘
                       TOML/env/app-policy resolution
                                     │ embedded data by default
                                     ▼
                   ┌────────────────────────────────────┐
                   │  crates/typeflow-core/data/        │
                   │   en.ngrams.bin / uk.ngrams.bin    │  ← compile-time inputs
                   │   en.dict.fst   / uk.dict.fst      │
                   │   en.dict-prefix.bin / uk...       │
                   └────────────────────────────────────┘
                                     ▲
                                     │ produces
                   ┌─────────────────┴──────────────────┐
                   │  typeflow-data (xtask)             │
                   │   downloads OPUS + hermitdave      │
                   │   counts char n-grams              │
                   │   builds frequency FSTs            │
                   └────────────────────────────────────┘
```

## Crates

### `typeflow-core`

Pure Rust. Hot path has no I/O; startup can deserialize embedded data or load an
external pack directory. Contains:

- `types.rs` — public API data types: layouts, input events, composition actions,
  decisions, scores, host context, and config.
- `keyboard.rs` — physical key positions, keyboard maps, reverse mapping, and
  layout rendering helpers.
- `engine.rs` — state machine and composition protocol implementation.
- `score.rs` — n-gram + dictionary scoring and dictionary-evidence checks.
- `data.rs` — language model, dictionary lookup, embedded artifacts, and pack
  loading/validation.
- `PhysicalKey` — 34 enum variants (26 ANSI letters + `` ` `` `[` `]` `;` `'` `,` `.` `\`).
  `PhysicalKey::from_char` maps English-US characters back to key positions;
  loaded `KeyboardMap`s handle secondary-layout reverse mapping.
- `KeyboardMap` / `LanguagePack` — runtime data for one side of the pair:
  layout rendering, n-gram model, dictionary FST, manifest validation, and
  model metadata. English is fixed; the secondary side can be embedded or
  loaded from an installed external pack.
- `InputEvent` — `Letter(LetterEvent)` / `Literal(char)` / `Backspace` /
  `EndToken` / `HostBypass`. `Literal` is for digits, separators, and
  characters that are not physical letters in either loaded layout; the engine
  treats it as a hard composition boundary. Punctuation
  keys that are letters in the secondary layout stay as `LetterEvent`s, with an
  English-token boundary heuristic for normal punctuation after resolved
  English words. Modifier shortcuts (Cmd/Ctrl/Opt) come in as `HostBypass`.
  The host decides what counts as `EndToken` (typically space/tab/return).
- `HostSurfaceFacts` / `HostInputPolicy` — Swift supplies host facts such as
  secure-input state, bundle id, input-client class, and focused accessibility
  metadata. Rust classifies the surface. Terminal-like surfaces, secure fields,
  and fully disabled apps block both automatic composition and standalone Option
  conversion. Auto-disabled apps block only automatic processing.
- `HostContext` — persistent engine bypass state derived from host policy.
  Secure input, terminal-like surfaces, fully disabled apps, and unavailable
  host config are full bypasses: normal key processing returns
  `CompositionAction::Bypass`
  and clears its token. Apps in `disable_auto_bundle_ids` use automatic-switching
  disabled mode instead: Rust still owns composition in the current layout, but
  it does not score or switch automatically.
- `Engine::process(InputEvent) -> CompositionOutput` — the only loop the host
  runs.
- `CompositionAction` — what the host should do in response: `Bypass`,
  `Render { text, layout }`, `Commit { text, consume_event }`, or
  `Clear { consume_event }`.
- `docs/invariants.md` — the stable core/host contract. If this conflicts
  with a CLI convenience behavior, the invariants doc wins.
- `data::LanguageBundle` — n-gram models + FST dictionaries, normally loaded
  from `embedded()` via `include_bytes!`. `from_secondary_pack_dir(path)` loads
  an installed pack for the non-English side. `from_data_dir(path)` remains a
  dev override for testing rebuilt artifacts without changing the binary.
- `EngineConfig` — every tuning knob (see `docs/engine.md`).

Tests live in `crates/typeflow-core/src/lib.rs#tests` and use `LanguageBundle::for_testing`
to drive the engine with synthetic inline word lists — no on-disk artifacts required.

### `typeflow-host-config`

Rust host-policy/config layer shared by the CLI and FFI host. It owns TOML
config parsing, environment overrides, default config paths, app disable policy,
and host-surface classification from bundle id / secure-input / Accessibility
facts. It depends on `typeflow-core`; `typeflow-core` does not depend on it.

### `typeflow-data` (xtask)

A binary that produces data artifacts. With no arguments it rebuilds the embedded
EN/UK artifacts. With `build-pack <spec.toml> --out <dir>` it builds an
installable external secondary-language pack. Cached downloads live under
`target/typeflow-data-cache/` and are reused across runs. Embedded source
downloads are pinned by byte count and SHA-256; external pack specs can pin
their own `corpus_sha256` and `dictionary_sha256`.

Inputs:

- **OpenSubtitles2018** monolingual text dumps from OPUS for char n-gram counts.
  - EN (3.66 GB gz, sampled to ~200 MB plaintext — n-grams converge way before that).
  - UK (~17 MB gz, full).
- **hermitdave/FrequencyWords** — pre-tokenized word + frequency lists derived
  from the same OpenSubtitles dump.

Outputs:

- `{en,uk}.ngrams.bin` — Typeflow n-gram artifact `CompiledLanguageData`
  (sorted bigrams + trigrams with log-probabilities + smoothing floors).
- `{en,uk}.dict.fst` — BurntSushi `fst::Map` (word → frequency).
- `{en,uk}.dict-prefix.bin` — serialized capped prefix sums used by hot-path
  dictionary evidence scoring.
- External packs: `pack.toml`, `ngrams.bin`, `dict.fst`, `dict-prefix.bin`.
  Spec details live in `docs/pack-spec.md`.

Data-source attribution and license notes live in `NOTICE.md`.

### `typeflow-cli`

The interactive binary. Subcommands are all driven by the same engine:

| Subcommand | Behaviour |
|---|---|
| `typeflow type <KEYS>...` | per-keystroke trace + final score breakdown |
| `typeflow stream` | stdin tokens → TSV decisions with active pack ids |
| `typeflow predict [--json] <KEYS>` | one-shot decision, pipe-friendly |
| `typeflow convert <KEYS>` | force-convert one token to the opposite layout |
| `typeflow repl` | `crossterm` raw-mode TTY, type live, see live scores + simulated committed text |
| `typeflow eval [--generated [N] \| <tsv>]` | run hard-case, generated dictionary, or external labeled corpus checks |
| `typeflow model` | print language-pack metadata and fingerprints |
| `typeflow pack install/list/use/inspect` | external language-pack workflow |
| `typeflow config init/show` | manage `~/.config/typeflow/config.toml` |

Performance checks are Cargo benchmarks, not CLI subcommands:

```sh
cargo bench -p typeflow-core
cargo bench -p typeflow-ffi
```

### `typeflow-ffi`

C ABI for the macOS bundle. Exports:

- `typeflow_engine_new_embedded()` / `typeflow_engine_new_from_data_dir(path)` /
  `typeflow_engine_new_from_pack_dir(path)` / `typeflow_engine_free`
- `_with_config(...)` constructor variants plus `typeflow_engine_default_config(...)`
  for hosts that need runtime tuning while keeping config validation in Rust.
- `typeflow_engine_process(engine, TfEvent, *out TfComposition)` — the hot path
- `typeflow_engine_reset_token` / `typeflow_engine_set_host_context` /
  `typeflow_engine_current_layout`

Header at `crates/typeflow-ffi/include/typeflow.h` is generated with cbindgen
from `crates/typeflow-ffi/src/lib.rs` and checked in CI. Builds as both
`staticlib` and `cdylib` (`libtypeflow_ffi.dylib`).

`TfEvent` supports physical-key letters, literals by Unicode codepoint,
backspace, and end-token boundaries. The 4096-byte fixed `text` buffer in
`TfComposition` keeps the FFI lifetime-free: no Vec passed across the boundary,
Swift just copies bytes. See `docs/invariants.md` for the required ownership,
event, and composition semantics.

### `macos/`

Staticlib bridge smoke plus the current IMKInputController bundle.
Current files:

- `Makefile` builds `libtypeflow_ffi.a`, compiles Swift, and runs the smoke.
- `Package.swift` defines the SwiftPM `TypeflowKit`, staticlib smoke,
  registration helper, and input-method executable targets.
- `TypeflowFFI/include/module.modulemap` exposes the C ABI to Swift.
- `TypeflowFFI/include/typeflow_shim.h` includes the canonical Rust header and
  adds tiny C helpers for zeroed composition/events.
- `Sources/TypeflowKit/Engine.swift` wraps the opaque `TfEngine*` lifecycle and
  decodes `TfComposition`.
- `Sources/TypeflowKit/HostConfig.swift` owns only an opaque Rust
  `TfHostConfig*`. Rust parses and resolves `~/.config/typeflow/config.toml`,
  environment overrides, app disable policy, engine tuning, language id, and
  data/pack paths through the FFI. Swift only supplies host facts such as the
  frontmost macOS bundle id and secure-input state.
- `Sources/TypeflowKit/KeyCodeMap.swift` maps macOS ANSI virtual keycodes to
  Rust physical key indexes.
- `Sources/TypeflowSmoke/main.swift` verifies `ghsdbn` becomes `привіт` through
  the Rust static archive.
- `Sources/TypeflowInputMethod/InputController.swift` subclasses
  `IMKInputController`, collects host-surface facts, applies Rust input policy,
  handles raw keyDown and flagsChanged events, dispatches keyDown events to the
  Rust engine, binds standalone Option press/release to manual conversion, and
  applies `TypeflowCompositionAction` to `IMKTextInput` marked text or final
  commit.
- `Sources/TypeflowInputMethod/main.swift` starts the `IMKServer`.
- `Sources/TypeflowRegister/main.swift` calls `TISRegisterInputSource` after
  install, enables/selects the Typeflow input method, and writes the
  `com.apple.HIToolbox` `AppleEnabledInputSources` records that System Settings
  reads.
- `Resources/Info.plist` defines the Typeflow input method bundle. The
  selectable TIS source id is `io.github.nnnickg.typeflow.inputmethod.Typeflow`.
  The bundle id intentionally contains `.inputmethod.` because TIS depends on
  that old naming convention.

`make -C macos bundle` builds and ad-hoc signs `Typeflow.app`.
`make -C macos install-user` installs it under `~/Library/Input Methods/` and
registers/enables it with TIS. Local manual host testing has verified real text
  input, external pack loading, app disable policy, and standalone Option manual
conversion. Remaining IMK validation work:

1. Run a broader app matrix: TextEdit, browser fields, Notes, Mail, Slack, and
   code editors when not disabled.
2. Re-test secure/password fields. The host checks Carbon secure event input,
   but not every app is equally disciplined.
3. Keep an eye on IMK dispatch mode. Standalone Option requires raw
   `handleEvent`/`NSEvent` delivery; the decomposed
   `inputText:key:modifiers:client:` path does not deliver modifier-only events.

## Data flow during a keystroke

1. User presses a key. macOS sends a raw `NSEvent` to the IMK controller (or in
   CLI mode, the user types into the REPL). Standalone Option arrives as
   `flagsChanged` and is handled by the host as manual conversion.
2. Host translates keyDown events to a `TfEvent` (or constructs an `InputEvent`
   directly in Rust). Standalone Option does not become a `TfEvent`; it calls
   `typeflow_engine_force_switch_token`.
3. The macOS host asks Rust to create the engine from the resolved
   `TfHostConfig`. `language.secondary = "uk"` uses embedded Ukrainian; any
   other secondary language loads from the installed pack directory. Dev builds
   can point at a rebuilt data directory.
4. `Engine::process(event)`:
   - Pushes the event onto the internal token (a `Vec<LetterEvent>`) unless
     the token has exceeded `max_token_len` and is bypassing until a boundary.
   - Updates both layout candidates from the token (English string, secondary string).
   - `score_layout` for each language: bigram + trigram + dict bonuses.
   - `decide` checks `min_token_len`, `disable_on_internal_caps`, then picks
     the winning layout if its margin clears the required confidence threshold.
5. Returns a `CompositionOutput { candidates, score, decision, action }`.
6. Host applies the `action`:
   - `Render { text, layout }` → redraw active marked/overlay composition.
   - `Commit { text, consume_event }` → insert finalized text once.
   - `Clear { consume_event }` → clear active composition.
   - `Bypass` → let the host app process the event normally.

## Non-goals

- A CGEventTap "global hook" architecture. Backspace-and-retype is the wrong
  shape; we want a real input source so the host app sees correct text the first time.
- Layouts needing keys outside the current 34-position model.
- Cloud inference of any kind.
- A preferences UI before the engine is calibrated.
