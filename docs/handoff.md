# Handoff

Current working state. If you're picking this up cold, read this end-to-end and
you'll have everything.

## What's done

### Engine (typeflow-core) — works on real data

- `PhysicalKey` covers 34 positions (26 ANSI letters + `` ` `` `[` `]` `;`
  `'` `,` `.` `\`). The backslash position exists for Ukrainian `ґ`.
- `PhysicalKey::from_char` only covers the fixed English-US side. Text-driven
  callers that need Cyrillic/custom secondary reverse mapping must use
  `LanguageBundle::letter_event_from_char`.
- `InputEvent::Letter / Literal / Backspace / EndToken` is the unified entry.
  Hosts decide what counts as `EndToken` (space, tab, return, punctuation
  outside the letter set, focus loss, etc.). `InputEvent::HostBypass` covers
  modifier shortcuts.
- `Layout` is now `English` / `Secondary`; language-specific layout enum
  variants are gone.
- `HostContext` lets Swift/IMK bypass the engine for secure input fields and
  excluded apps.
- `LanguagePack` now carries the keyboard map, language id/display/script,
  keyboard layout id, n-gram model, dictionary FST, manifest validation, and
  runtime metadata. English is still fixed; the second side can be embedded or
  loaded from an installed external pack.
- Real scoring: char bigram + char trigram log-probabilities from
  OpenSubtitles2018 + dictionary FST (exact + prefix bonuses) from
  hermitdave/FrequencyWords.
- Length-normalized score + per-feature weights. All knobs configurable.
- N-gram-only switches require a stricter margin, so tokens like `http` do not
  flip just because the secondary layout is the less-bad candidate.
- `disable_on_internal_caps` blocks switching on camelCase / PascalCase tokens.
- Literal digits/separators bypass the current token (`abc123`, URLs, paths,
  snake_case). Punctuation-position keys remain physical letters for secondary
  layouts, but English punctuation on those keys terminates a token once the
  current token has already resolved as English.
- `force_switch_token()` exists for Punto-style manual correction.
- Unit tests cover synthetic in-memory bundles, pack parser failures, malformed
  n-gram/FST artifacts, weird Unicode literals, host bypass, and devops/security
  false positives.
- `docs/invariants.md` defines the core/host contract for token state,
  actions, FFI ownership, and calibration boundaries.

### Data pipeline (typeflow-data) — works

`cargo run --release -p typeflow-data` downloads ~3.7 GB into
`target/typeflow-data-cache/` (resumable — won't re-download on subsequent
runs), processes everything, and writes four artifacts to
`crates/typeflow-core/data/`. Those artifacts are compile-time inputs embedded
into release binaries with `include_bytes!`. The embedded pair is English plus
Ukrainian; other secondary languages use the external language-pack workflow.
The raw subtitle cache is never needed at runtime.

`cargo run --release -p typeflow-data -- build-pack <spec.toml> --out <dir>`
builds an external pack directory (`pack.toml`, `ngrams.bin`, `dict.fst`) from
local files or HTTP/HTTPS corpus/dictionary inputs. The spec format is documented
in `docs/pack-spec.md`.

### CLI (typeflow-cli) — done

Subcommands all driving the same engine:

- `typeflow type <KEYS>...` — per-keystroke trace + final breakdown
- `typeflow stream` — stdin lines → TSV decisions
- `typeflow predict [--json] <KEYS>` — one-shot, pipeable
- `typeflow convert <KEYS>` — force-convert one token to the opposite layout
- `typeflow eval [<tsv>]` — run hard-case or labeled corpus checks
- `typeflow model` — print language-pack metadata/fingerprints
- `typeflow pack install/list/use/inspect` — external language-pack workflow.
  Installed packs are directories containing `pack.toml`, `ngrams.bin`, and
  `dict.fst`; the release binary remains standalone.
- `typeflow repl` — interactive raw-mode TTY with live score panel and
  "what you would see in TextEdit" simulated commit buffer
- `typeflow config init/show` — manage `~/.config/typeflow/config.toml`

`--config <path>`, `$TYPEFLOW_CONFIG`, and `~/.config/typeflow/config.toml` are
all honored, in that override precedence.

Performance checks live under Cargo's benchmark harness:

```sh
cargo bench -p typeflow-core
cargo bench -p typeflow-ffi
```

### FFI (typeflow-ffi) — surface ready

Header at `crates/typeflow-ffi/include/typeflow.h`. Builds as `staticlib`,
`cdylib`, and `rlib`. `macos/` has a Swift staticlib smoke target that links
`libtypeflow_ffi.a`, calls the C ABI, and verifies `ghsdbn -> привіт`. Release
hosts should use `typeflow_engine_new_embedded()` or
`typeflow_engine_new_embedded_with_config(...)`.
`typeflow_engine_new_from_data_dir(...)` is a dev override for testing rebuilt
model artifacts. `typeflow_engine_new_from_pack_dir(...)` loads embedded English
plus one installed secondary language pack. FFI exposes `TF_EVENT_LITERAL`,
`TF_LAYOUT_SECONDARY`, `_with_config(...)` constructors,
`typeflow_engine_default_config(...)`, modifier-bypass bits, and
`typeflow_engine_set_host_context(...)`.

### macOS bridge (`macos/`) — working IMK bundle

`make -C macos smoke` builds the Rust `typeflow-ffi` static archive, compiles
Swift with the local module map, links against `libtypeflow_ffi.a`, and runs a
host-buffer smoke test.

`make -C macos bundle` builds and ad-hoc signs `Typeflow.app`. The executable
starts an `IMKServer` from `Info.plist`, exposes `TypeflowInputController`,
receives raw `NSEvent` keyDown/flagsChanged events, maps ANSI keycodes to Rust
physical key indexes, calls the FFI, and applies
`TypeflowAction` through `IMKTextInput.insertText(_:replacementRange:)`.
The Swift host reads the same config path as the CLI (`~/.config/typeflow/config.toml`):
engine knobs, `language.secondary`, `packs.directory`, `data.directory`, and
`apps.exclude_bundle_ids`. Environment overrides for data/pack directories take
precedence over TOML, matching the CLI. `language.secondary = "uk"` uses embedded Ukrainian;
other values load `~/Library/Application Support/Typeflow/packs/<id>` unless
overridden. Standalone Option press/release is hardcoded as manual conversion;
Option+another key cancels the pending manual conversion and passes through as
normal app input. Do not re-add the `inputText:key:modifiers:client:` path
without checking standalone Option: macOS chose that decomposed path and stopped
delivering modifier-only transitions.

`make -C macos install-user` copies the bundle to `~/Library/Input Methods/`,
calls `TISRegisterInputSource`, enables the parent input method plus the visible
Typeflow mode, and writes the `com.apple.HIToolbox`
`AppleEnabledInputSources` entries that System Settings reads. TIS sees:

- `io.github.nnnickg.typeflow.inputmethod.Typeflow`
  (`TISTypeKeyboardInputMethodModeEnabled`)
- `io.github.nnnickg.typeflow.inputmethod.Typeflow.Main`
  (`TISTypeKeyboardInputMode`, language `mul`, input mode `Typeflow`)

Manual host testing has verified:

- External secondary pack loading through `language.secondary`.
- App exclusions via `apps.exclude_bundle_ids` for iTerm2 and Zed.
- Normal replacement in real text fields.
- Standalone Option manual conversion in real text fields.

The host also resets token state when the input client changes, selected text
is active, or the caret location no longer matches the previous action's
predicted location.

Files:

- `macos/TypeflowFFI/include/module.modulemap`
- `macos/TypeflowFFI/include/typeflow_shim.h`
- `macos/Sources/TypeflowKit/Engine.swift`
- `macos/Sources/TypeflowKit/HostConfig.swift`
- `macos/Sources/TypeflowKit/KeyCodeMap.swift`
- `macos/Sources/TypeflowSmoke/main.swift`
- `macos/Sources/TypeflowInputMethod/InputController.swift`
- `macos/Sources/TypeflowInputMethod/main.swift`
- `macos/Sources/TypeflowRegister/main.swift`
- `macos/Resources/Info.plist`
- `macos/Resources/PkgInfo`
- `macos/Resources/Typeflow.png`
- `macos/Makefile`

### CI — enabled

`.github/workflows/ci.yml` runs fmt, workspace tests, clippy with warnings
denied, release workspace tests, benchmark compilation, release CLI/FFI build,
Swift staticlib smoke, signed IMK bundle build, and release CLI smoke against
embedded Ukrainian on macOS.

## What's NOT done

### Broader macOS compatibility pass

The bundle works locally as a single visible Typeflow mode that emits both Latin
and Cyrillic. It still needs a broader app matrix before calling the macOS host
stable: TextEdit, Safari/Chrome text fields, Notes, Mail, Slack, VS Code/Zed
when not excluded, and password fields. Keep ABC installed as the system
fallback; macOS will not let a user remove every plain keyboard layout while an
IMK input method is installed.

### Regression corpus + calibration

`typeflow eval` still runs the small built-in smoke set. `typeflow eval
--generated [limit-per-layout]` now builds a larger regression corpus from the
loaded dictionaries: top EN words expect English, top secondary words are
rendered back to physical-key strings and expect secondary unless that key
string is an exact English dictionary word. Those ambiguous generated secondary
cases are skipped and counted. External TSVs are still supported with
`keys<TAB>expected-layout`. Eval output now includes accuracy, confusion counts,
false positives/negatives, failing token lengths, and a bounded failure sample.

Defaults (especially `confidence_margin = 1.0`) are still an educated guess.
Keep running generated eval at useful limits, add hand-curated hard cases for
ambiguous short tokens / code identifiers / mixed-script names, then tune until
accuracy is north of 95%.
See `docs/calibration.md` for the current ambiguity policy.

### Host-driven behavior

The macOS host now enforces `apps.exclude_bundle_ids` and checks Carbon secure
event input before processing key events. Both paths set the FFI host context,
clear token state, and return the event to the client app unchanged. The host
also loads scoring knobs, active secondary language, pack directory, and data
directory from the same config file as the CLI.

Manual convert is hardcoded in the macOS host as a standalone Option
press/release. It is intentionally not configurable. Option+another key cancels
the pending manual convert and stays a normal app shortcut/input sequence.

## Outstanding limitations to be aware of

1. **Once flipped, layout sticks.** When the engine switches to secondary
   mid-token, it stays there for the rest of the token. Probably fine for
   real use; flag if calibration finds nasty cases.
2. **Dictionary noise.** OPUS / hermitdave secondary lists may contain Latin proper
   names and English loanwords. Words like "amazon" appear in BOTH dictionaries
   with non-trivial counts, weakening the dict signal on certain tokens.
   Filter step in `typeflow-data` would help.
3. **Secure-field detection is host-signal limited.** The IMK layer checks
   Carbon secure event input and app exclusions before processing keys. If an
   app does not enable secure event input for a sensitive field, Typeflow cannot
   infer that from text content without doing exactly the kind of inspection we
   should avoid.
4. **Sample asymmetry.** EN n-grams come from ~200 MB of OPUS, secondary packs
   may use different corpus sizes. The smoothed floors and overall scale differ between
   languages. Probably fine for PoC; revisit if calibration finds bias.

## Where to look first

If you're debugging engine behaviour:

- `crates/typeflow-core/src/engine.rs` — state transitions, `decide`,
  `step_letter`, backspace/literal handling.
- `crates/typeflow-core/src/score.rs` — `score_layout`,
  `has_dictionary_evidence`.
- `crates/typeflow-core/src/keyboard.rs` — physical key mapping, keyboard maps,
  render/reverse-map helpers.
- `crates/typeflow-core/src/types.rs` — public event/action/config/score types.
- `crates/typeflow-core/src/data.rs` — `LanguageModel`, `dict_lookup`,
  `LanguageBundle::for_testing`.
- `crates/typeflow-cli/src/main.rs` — REPL is the fastest way to feel the
  engine; `cmd_predict` is the simplest scoring path.
- Run `typeflow type <whatever>` for a per-keystroke trace.

If you're building the IMK bundle:

- `docs/invariants.md` — read this first. It is the host contract.
- `docs/artifact-format.md` — embedded artifact and external pack compatibility
  policy.
- `docs/panic-unsafe-audit.md` — FFI unsafe boundary and panic/indexing audit.
- `docs/release-verification.md` — release artifact checks and the dylib
  install-name caveat the packaging script must handle.
- `macos/Sources/TypeflowKit/Engine.swift` — Swift wrapper already calling the
  staticlib through the C ABI.
- `macos/Sources/TypeflowInputMethod/InputController.swift` — current IMK
  raw keyDown/flagsChanged event path, host context, standalone Option manual
  conversion, and action application.
- `crates/typeflow-ffi/include/typeflow.h` — exact ABI to consume.
- `crates/typeflow-ffi/src/lib.rs` — Rust side of the bridge; understand
  `TfEvent` / `TfAction` / `typeflow_engine_process` before writing Swift.
- `crates/typeflow-ffi/tests/abi_smoke.rs` — public ABI host-buffer simulation
  that Swift should mirror.
- `docs/engine.md#the-action-protocol-host-contract` — extra explanation for
  the action protocol. The invariants doc is the source of truth.

If you're tuning thresholds:

- `docs/engine.md#calibration-how-to-tune` — what to do.
- `docs/calibration.md` — what ambiguous generated cases mean.
- `~/.config/typeflow/config.toml` — where to do it.
- `typeflow config show` — verify what the engine is actually loading.

## Open questions for the next agent / next session

1. **Embedding strategy.** Should the IMK bundle ship `include_bytes!`-embedded
   data (~10 MB binary) or load from `Bundle.main.resourcePath` as files? File
   loading is cheaper to update; embedding is simpler.
2. **Dictionary expansion.** hermitdave's lists include only attested surface
   forms from OPUS. For rare secondary-language inflections this misses obvious words.
   Worth merging in Hunspell expansions before the regression corpus pass?
3. **Score calibration target.** Current policy weights false-positive
   switches as worse than false-negative no-switches. Revisit only if real
   typing sessions show manual conversion is too frequent.
4. **Multi-app config.** Does the user want different thresholds per app
   (e.g. more conservative in code editors)? Schema-wise it's
   `[apps.com.googlecode.iterm2.engine]` — defer until calibration is done.
