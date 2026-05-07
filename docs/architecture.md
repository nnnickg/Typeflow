# Architecture

## High-level shape

```text
                   ┌────────────────────────────────────┐
                   │  macOS InputMethodKit bundle       │
                   │  (Swift/AppKit IMKInputController) │
                   └─────────────┬──────────────────────┘
                                 │ keyDown / keyCode / shift
                                 ▼
┌────────────────────┐   ┌───────────────────────────────┐   ┌────────────────────┐
│  typeflow-cli      │──▶│  typeflow-core                │◀──│  typeflow-ffi      │
│  type / stream     │   │   - PhysicalKey, InputEvent   │   │  C ABI for IMK     │
│  repl / predict    │   │   - Engine.process()          │   │  (Swift IMK host) │
│  pack / config     │   │   - LanguageBundle scoring    │   │                    │
└────────────────────┘   └───────────┬───────────────────┘   └────────────────────┘
                                     │ embedded data by default
                                     ▼
                   ┌────────────────────────────────────┐
                   │  crates/typeflow-core/data/        │
                   │   en.ngrams.bin / uk.ngrams.bin    │  ← compile-time inputs
                   │   en.dict.fst   / uk.dict.fst      │
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

- `types.rs` — public API data types: layouts, input events, actions,
  decisions, scores, host context, and config.
- `keyboard.rs` — physical key positions, keyboard maps, reverse mapping, and
  layout rendering helpers.
- `engine.rs` — state machine and action protocol implementation.
- `score.rs` — n-gram + dictionary scoring and dictionary-evidence checks.
- `data.rs` — language model, dictionary lookup, embedded artifacts, and pack
  loading/validation.
- `PhysicalKey` — 34 enum variants (26 ANSI letters + `` ` `` `[` `]` `;` `'` `,` `.` `\`).
  Bidirectional `from_char` accepts both Latin and Cyrillic input.
- `KeyboardMap` / `LanguagePack` — runtime data for one side of the pair:
  layout rendering, n-gram model, dictionary FST, manifest validation, and
  model metadata. English is fixed; the secondary side can be embedded or
  loaded from an installed external pack.
- `InputEvent` — `Letter(LetterEvent)` / `Literal(char)` / `Backspace` /
  `EndToken` / `HostBypass`. `Literal` is for digits, punctuation, separators,
  and any other non-letter character; the engine treats it as a hard token
  boundary that also commits the char. Modifier shortcuts (Cmd/Ctrl/Opt) come
  in as `HostBypass`. The host decides what counts as `EndToken` (typically
  space/tab/return).
- `HostContext` — persistent host-level state: secure input fields and
  excluded foreground apps. While either is set the engine returns
  `Action::Keep` and clears its token.
- `Engine::process(InputEvent) -> EngineOutput` — the only loop the host runs.
- `Action` — what the host should do in response: `Keep`, `Commit(char)`,
  `ReplaceToken { old_len, replacement, layout }`, `ResetToken`.
- `docs/invariants.md` — the stable core/host contract. If this conflicts
  with a CLI convenience behavior, the invariants doc wins.
- `data::LanguageBundle` — n-gram models + FST dictionaries, normally loaded
  from `embedded()` via `include_bytes!`. `from_secondary_pack_dir(path)` loads
  an installed pack for the non-English side. `from_data_dir(path)` remains a
  dev override for testing rebuilt artifacts without changing the binary.
- `EngineConfig` — every tuning knob (see `docs/engine.md`).

Tests live in `crates/typeflow-core/src/lib.rs#tests` and use `LanguageBundle::for_testing`
to drive the engine with synthetic inline word lists — no on-disk artifacts required.

### `typeflow-data` (xtask)

A binary that produces data artifacts. With no arguments it rebuilds the embedded
EN/UK artifacts. With `build-pack <spec.toml> --out <dir>` it builds an
installable external secondary-language pack. Cached downloads live under
`target/typeflow-data-cache/` and are reused across runs.

Inputs:

- **OpenSubtitles2018** monolingual text dumps from OPUS for char n-gram counts.
  - EN (3.66 GB gz, sampled to ~200 MB plaintext — n-grams converge way before that).
  - UK (~17 MB gz, full).
- **hermitdave/FrequencyWords** — pre-tokenized word + frequency lists derived
  from the same OpenSubtitles dump.

Outputs:

- `{en,uk}.ngrams.bin` — `bincode`-serialized `CompiledLanguageData`
  (sorted bigrams + trigrams with log-probabilities + smoothing floors).
- `{en,uk}.dict.fst` — BurntSushi `fst::Map` (word → frequency).
- External packs: `pack.toml`, `ngrams.bin`, `dict.fst`. Spec details live in
  `docs/pack-spec.md`.

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

C ABI for the future macOS bundle. Exports:

- `typeflow_engine_new_embedded()` / `typeflow_engine_new_from_data_dir(path)` /
  `typeflow_engine_new_from_pack_dir(path)` / `typeflow_engine_free`
- `_with_config(...)` constructor variants plus `typeflow_engine_default_config(...)`
  for hosts that need runtime tuning without duplicating CLI TOML parsing.
- `typeflow_engine_process(engine, TfEvent, *out TfAction)` — the hot path
- `typeflow_engine_reset_token` / `typeflow_engine_set_host_context` /
  `typeflow_engine_current_layout`

Header at `crates/typeflow-ffi/include/typeflow.h`. Builds as both `staticlib`
and `cdylib` (`libtypeflow_ffi.dylib`).

`TfEvent` supports physical-key letters, literals by Unicode codepoint,
backspace, and end-token boundaries. The 4096-byte fixed `replace_text` buffer
in `TfAction` keeps the FFI lifetime-free: no Vec passed across the boundary,
Swift just copies bytes. See `docs/invariants.md` for the required ownership,
event, and action semantics.

### `macos/`

Staticlib bridge smoke plus the current minimal IMKInputController bundle.
Current files:

- `Makefile` builds `libtypeflow_ffi.a`, compiles Swift, and runs the smoke.
- `TypeflowFFI/include/module.modulemap` exposes the C ABI to Swift.
- `TypeflowFFI/include/typeflow_shim.h` includes the canonical Rust header and
  adds tiny C helpers for zeroed actions/events.
- `Sources/TypeflowKit/Engine.swift` wraps the opaque `TfEngine*` lifecycle and
  decodes `TfAction`.
- `Sources/TypeflowKit/KeyCodeMap.swift` maps macOS ANSI virtual keycodes to
  Rust physical key indexes.
- `Sources/TypeflowSmoke/main.swift` verifies `ghsdbn` becomes `привіт` through
  the Rust static archive.
- `Sources/TypeflowInputMethod/InputController.swift` subclasses
  `IMKInputController`, handles keyDown events, dispatches to the Rust engine,
  and applies `TfAction` to `NSTextInputClient`.
- `Sources/TypeflowInputMethod/main.swift` starts the `IMKServer`.
- `Sources/TypeflowRegister/main.swift` calls `TISRegisterInputSource` after
  install, enables the parent input method plus its visible Ukrainian mode, and
  writes the `com.apple.HIToolbox` `AppleEnabledInputSources` records that
  System Settings reads.
- `Resources/Info.plist` defines a mode-enabled input method bundle with one
  visible Ukrainian mode. The `InputModeID` is `Typeflow`; the selectable TIS source id is
  `io.github.nnnickg.typeflow.inputmethod.Typeflow.Ukrainian`. The bundle id
  intentionally contains `.inputmethod.` because TIS depends on that old naming
  convention.

`make -C macos bundle` builds and ad-hoc signs `Typeflow.app`.
`make -C macos install-user` installs it under `~/Library/Input Methods/` and
registers/enables it with TIS. It still needs manual host testing. The remaining IMK
work:

1. Confirm System Settings exposes the source and TextEdit receives correct
   output.
2. Validate the single-mode Latin+Cyrillic output path on current macOS.
3. Add host config handling for excluded apps, secure input, and manual convert.

## Data flow during a keystroke

1. User presses key. macOS sends `keyDown:` to the IMK controller (or in CLI
   mode, the user types into the REPL).
2. Host translates the event to a `TfEvent` (or constructs an `InputEvent`
   directly in Rust).
3. The host creates the engine from the embedded bundle in release builds. Dev
   builds can point at a rebuilt data directory.
4. `Engine::process(event)`:
   - Pushes the event onto the internal token (a `Vec<LetterEvent>`) unless
     the token has exceeded `max_token_len` and is bypassing until a boundary.
   - Updates both layout candidates from the token (English string, secondary string).
   - `score_layout` for each language: bigram + trigram + dict bonuses.
   - `decide` checks `min_token_len`, `disable_on_internal_caps`, then picks
     the winning layout if its margin clears the required confidence threshold.
5. Returns an `EngineOutput { candidates, score, decision, action }`.
6. Host applies the `action`:
   - `Commit(c)` → insert one char in the current layout.
   - `ReplaceToken { old_len, replacement, layout }` → delete trailing
     `old_len` chars, insert `replacement`. Engine just flipped layouts.
   - `ResetToken` / `Keep` → host-side bookkeeping only.

## Non-goals

- A CGEventTap "global hook" architecture. Backspace-and-retype is the wrong
  shape; we want a real input source so the host app sees correct text the first time.
- Layouts needing keys outside the current 34-position model.
- Cloud inference of any kind.
- A preferences UI before the engine is calibrated.
