# Handoff

State of the project as of the last commit. If you're picking this up cold,
read this end-to-end and you'll have everything.

## What's done

### Engine (typeflow-core) — works on real data

- `PhysicalKey` covers 34 positions (26 ANSI letters + `` ` `` `[` `]` `;`
  `'` `,` `.` `\`). The backslash position exists for Ukrainian `ґ`.
- `PhysicalKey::from_char` is bidirectional — accepts both Latin and Cyrillic
  characters and reverse-maps to the underlying physical key.
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
  snake_case).
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
`cdylib`, and `rlib`. The Swift IMK bundle isn't built yet, but `macos/` now
has a Swift staticlib smoke target that links `libtypeflow_ffi.a`, calls the C
ABI, and verifies `ghsdbn -> привіт`. Release hosts should use
`typeflow_engine_new_embedded()` or
`typeflow_engine_new_embedded_with_config(...)`.
`typeflow_engine_new_from_data_dir(...)` is a dev override for testing rebuilt
model artifacts. `typeflow_engine_new_from_pack_dir(...)` loads embedded English
plus one installed secondary language pack. FFI exposes `TF_EVENT_LITERAL`,
`TF_LAYOUT_SECONDARY`, `_with_config(...)` constructors,
`typeflow_engine_default_config(...)`, modifier-bypass bits, and
`typeflow_engine_set_host_context(...)`.

### macOS bridge (`macos/`) — staticlib smoke + minimal IMK bundle build

`make -C macos smoke` builds the Rust `typeflow-ffi` static archive, compiles
Swift with the local module map, links against `libtypeflow_ffi.a`, and runs a
host-buffer smoke test.

`make -C macos bundle` builds and ad-hoc signs `Typeflow.app`. The executable
starts an `IMKServer` from `Info.plist`, exposes `TypeflowInputController`, maps
ANSI keycodes to Rust physical key indexes, calls the FFI, and applies
`TypeflowAction` through `NSTextInputClient.insertText(_:replacementRange:)`.
`make -C macos install-user` copies the bundle to `~/Library/Input Methods/`,
calls `TISRegisterInputSource`, and enables the parent input method plus the
visible Ukrainian mode. It also writes the `com.apple.HIToolbox`
`AppleEnabledInputSources` entries that System Settings reads for mode-enabled
input methods. TIS sees:

- `io.github.nnnickg.typeflow.inputmethod.Typeflow`
  (`TISTypeKeyboardInputMethodModeEnabled`)
- `io.github.nnnickg.typeflow.inputmethod.Typeflow.Ukrainian`
  (`TISTypeKeyboardInputMode`, language `uk`)

This is build/register/TIS-discovery verified, not manually host-tested in
TextEdit yet.

Files:

- `macos/TypeflowFFI/include/module.modulemap`
- `macos/TypeflowFFI/include/typeflow_shim.h`
- `macos/Sources/TypeflowKit/Engine.swift`
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

### macOS manual IMK validation — not done yet

The bundle builds and registers; the actual system integration still needs a
real GUI smoke.
Plan:

1. Run `make -C macos install-user`.
2. Reopen System Settings → Keyboard → Input Sources. Typeflow should be in
   the Ukrainian language bucket; search for `Typeflow` if the list is cached.
3. Type `ghsdbn` in TextEdit and verify `привіт`.
4. Confirm the one visible Ukrainian mode can emit both Latin and Cyrillic
   scripts.
5. If macOS rejects that in real apps, split into two paired modes/input
   sources.

The non-obvious part: macOS expects each input source to be tied to one primary
language/script. The current Punto-style approach exposes one Ukrainian mode and
lets the Rust engine emit either Latin or Cyrillic text. If that fails in real
apps, fall back to registering two paired modes/input sources and switching
between them programmatically (KeyKey-style).

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
Before the IMK bundle ships, run generated eval at useful limits, add
hand-curated hard cases for ambiguous short tokens / code identifiers /
mixed-script names, then tune until accuracy is north of 95%.
See `docs/calibration.md` for the current ambiguity policy.

### Host-driven config fields

These exist in the config schema, but the macOS host must decide when to call
the engine/FFI with the corresponding context:

- `apps.exclude_bundle_ids` — IMK should set `TF_CONTEXT_APP_EXCLUDED`.
- `hotkey.manual_convert` — engine/FFI support forced conversion; Swift side
  still needs a hotkey binding.

The scoring knobs are now available at the FFI layer through `TfEngineConfig`
and the `_with_config(...)` constructors. The host still has to decide where it
loads user preferences from.

## Outstanding limitations to be aware of

1. **Once flipped, layout sticks.** When the engine switches to secondary
   mid-token, it stays there for the rest of the token. Probably fine for
   real use; flag if calibration finds nasty cases.
2. **Dictionary noise.** OPUS / hermitdave secondary lists may contain Latin proper
   names and English loanwords. Words like "amazon" appear in BOTH dictionaries
   with non-trivial counts, weakening the dict signal on certain tokens.
   Filter step in `typeflow-data` would help.
3. **No focus-loss handling.** When the IMK bundle ships, `deactivateServer:`
   must reset the engine's token. Otherwise the next focused app inherits a
   stale buffer and gets weird replacements on its first letters.
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
  keyDown/action application path.
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

1. **macOS input source registration trick.** Does one visible Ukrainian IMK
   mode actually let us emit both Latin and Cyrillic in real apps, or do we
   need the two-paired-sources hack? Worth a 1-day spike before committing to
   the architecture.
2. **Embedding strategy.** Should the IMK bundle ship `include_bytes!`-embedded
   data (~10 MB binary) or load from `Bundle.main.resourcePath` as files? File
   loading is cheaper to update; embedding is simpler.
3. **Dictionary expansion.** hermitdave's lists include only attested surface
   forms from OPUS. For rare secondary-language inflections this misses obvious words.
   Worth merging in Hunspell expansions before the regression corpus pass?
4. **Score calibration target.** Current policy weights false-positive
   switches as worse than false-negative no-switches. Revisit only if real
   typing sessions show manual conversion is too frequent.
5. **Multi-app config.** Does the user want different thresholds per app
   (e.g. more conservative in code editors)? Schema-wise it's
   `[apps.com.googlecode.iterm2.engine]` — defer until calibration is done.
