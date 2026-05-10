# Project Status

Current working state. Typeflow is pre-alpha software; use it at your own risk,
keep a normal keyboard layout installed as a fallback, and expect behavior to
change before a stable release. Read this for implementation status, known
limitations, and the areas that still need validation before a stable release.

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
- `HostContext` lets Swift/IMK fully bypass secure input fields, terminal-like
  surfaces, and fully disabled apps. Apps with only automatic processing
  disabled still let Rust commit the current manual layout, but automatic
  scoring/switching is disabled.
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
  composition actions, FFI ownership, and calibration boundaries.

### Data pipeline (typeflow-data) — works

`cargo run --release -p typeflow-data` downloads ~3.7 GB into
`target/typeflow-data-cache/` (resumable — won't re-download on subsequent
runs), processes everything, and writes six artifacts to
`crates/typeflow-core/data/`. Those artifacts are compile-time inputs embedded
into release binaries with `include_bytes!`. The embedded pair is English plus
Ukrainian; other secondary languages use the external language-pack workflow.
N-gram counting batches lines across worker threads and counts packed `u64`
bigram/trigram keys, avoiding per-observation string allocation; strings are
only materialized when writing the artifact. The raw subtitle cache is never
needed at runtime.

`cargo run --release -p typeflow-data -- build-pack <spec.toml> --out <dir>`
builds an external pack directory (`pack.toml`, `ngrams.bin`, `dict.fst`,
`dict-prefix.bin`) from local files or HTTP/HTTPS corpus/dictionary inputs. The
spec format is documented in `docs/pack-spec.md`.

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
  `dict.fst`/`dict-prefix.bin`; the release binary remains standalone.
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

Header at `crates/typeflow-ffi/include/typeflow.h`, generated from Rust with
`cbindgen --config cbindgen.toml --crate typeflow-ffi`. CI verifies the checked-in
header is not stale. Builds as `staticlib`, `cdylib`, and `rlib`. `macos/` has a
Swift staticlib smoke target that links `libtypeflow_ffi.a`, calls the C ABI, and
verifies `ghsdbn -> привіт`. Release hosts should use `typeflow_engine_new_embedded()` or
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

`make -C macos swift-package` builds the SwiftPM target graph: `TypeflowKit` as
a library, the SwiftPM staticlib smoke executable, the TIS register helper, and
the input-method executable linked against `libtypeflow_ffi.a`.

`make -C macos bundle` builds and ad-hoc signs `Typeflow.app`. The executable
starts an `IMKServer` from `Info.plist`, exposes `TypeflowInputController`,
receives raw `NSEvent` keyDown/flagsChanged events, maps ANSI keycodes to Rust
physical key indexes, calls the FFI, and applies
`TypeflowCompositionAction` through the selected composition renderer and
`IMKTextInput.insertText(_:replacementRange:)` for final commits.

`make -C macos release-universal` builds arm64 and x86_64 Rust/Swift artifacts,
merges them with `lipo`, signs with hardened runtime when a Developer ID
identity is provided, and can notarize via `xcrun notarytool`.
Rust now resolves the macOS host config through `TfHostConfig`: engine knobs,
`language.secondary`, `packs.directory`, `data.directory`, environment
overrides, `apps.disable_bundle_ids`, `apps.disable_auto_bundle_ids`, and
`apps.direct_commit_bundle_ids`. Swift does not parse TOML. It keeps the opaque
config handle, asks Rust for resolved fields for logging/smoke tests, asks Rust
to create the engine from that config, and passes host-surface facts to Rust for
input-policy classification.
`language.secondary = "uk"` uses embedded Ukrainian; other values load
`~/Library/Application Support/Typeflow/packs/<id>` unless overridden.
Standalone Option press/release is hardcoded as manual conversion; Option+another
key cancels the pending manual conversion and passes through as normal app
input. Auto-disabled apps bypass automatic scoring/switching but still allow
explicit Option conversion in normal, non-secure text fields. After explicit
conversion, subsequent keys compose and commit in the selected manual layout
until the user converts back or the engine layout is reset.
Active composition uses native marked text by default so the text client owns
inline layout. Apps in `apps.direct_commit_bundle_ids` skip live host
composition and only mutate the document at the boundary commit.
Fully disabled apps and terminal-like surfaces bypass both automatic processing
and Option conversion. Terminal-like surfaces are detected from Rust-owned
policy using bundle ids plus focused accessibility metadata supplied by Swift.
The Rust heuristic intentionally ignores low-signal window titles and app names
to avoid false-disabling normal text fields. AX metadata is never queried on
input-source activation, is cached briefly on the key path, and uses a low
messaging timeout because synchronous Accessibility reads can otherwise freeze
typing. Embedded terminal-pane detection needs macOS Accessibility trust for the
Typeflow input method; terminal apps are still blocked by bundle id without AX
trust.
Do not re-add the `inputText:key:modifiers:client:` path without checking
standalone Option: macOS chose that decomposed path and stopped delivering
modifier-only transitions.

`make -C macos install-user` copies the bundle to `~/Library/Input Methods/`,
calls `TISRegisterInputSource`, enables the Typeflow input method, and writes
the `com.apple.HIToolbox`
`AppleEnabledInputSources` entries that System Settings reads. TIS sees:

- `io.github.nnnickg.typeflow.inputmethod.Typeflow`

Manual host testing has verified:

- External secondary pack loading through `language.secondary`.
- App disable policy via `apps.disable_bundle_ids` and
  `apps.disable_auto_bundle_ids`.
- Normal active composition and final commit in real text fields.
- Standalone Option manual conversion in real text fields.

The host also resets token/composition state when input stops, the client
changes, explicit composition commit runs, or host policy disables processing.

The input method logs slow host-path timings under the `Performance` category
for `processKey`, host policy/AX refresh, FFI calls, composition render/clear,
and `insertText`. To watch them live:

```sh
log stream --style compact --predicate 'subsystem == "io.github.nnnickg.typeflow.inputmethod.Typeflow" && category == "Performance"'
```

By default only calls over their local threshold are logged. To log every
measured call for a short profiling session:

```sh
launchctl setenv TYPEFLOW_PERF_LOG_ALL 1
pkill -x Typeflow
```

Unset it with `launchctl unsetenv TYPEFLOW_PERF_LOG_ALL` and restart Typeflow.

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
- `macos/Package.swift`
- `macos/Makefile`

### CI — enabled

`.github/workflows/ci.yml` runs fmt, workspace tests, clippy with warnings
denied, fuzz harness formatting/build checks, release workspace tests, benchmark
compilation, release CLI/FFI build, dependency deny/audit checks, generated FFI
header verification, LCOV coverage generation, Swift staticlib smoke, SwiftPM
builds, signed IMK bundle build, and release CLI smoke against embedded
Ukrainian on macOS.
Workspace lints deny `dbg!`, `todo!`, `unimplemented!`, `unwrap`, `expect`, and
`panic` in production targets; tests and benchmarks explicitly allow panic-style
assertion helpers where they are the right shape.

## Not Done Yet

### Supply-chain review provenance

CI now runs `cargo-deny` and `cargo-audit`, but `cargo-vet` is not configured.
Do not add a fake vet policy full of blanket exemptions just to make a badge
green. If vetting becomes a release gate, the next step is to initialize
`cargo-vet`, import trusted third-party audits where available, and explicitly
review/certify the remaining direct dependencies.

### Broader macOS compatibility pass

The bundle works as a Typeflow input method that emits both Latin and Cyrillic.
It still needs a broader app matrix before calling the macOS host stable:
TextEdit, Safari/Chrome text fields, Notes, Mail, Slack, VS Code/Zed when not
disabled, and password fields. Use `docs/host-test-matrix.md` as the release
gate for that pass. Keep ABC installed as the system fallback; macOS will not
let a user remove every plain keyboard layout while an IMK input method is
installed.

### Regression corpus + calibration

`typeflow eval` still runs the small built-in smoke set. `typeflow eval
--generated [limit-per-layout]` now builds a larger regression corpus from the
loaded dictionaries: top EN words expect English, top secondary words are
rendered back to physical-key strings and expect secondary unless that key
string is an exact English dictionary word. Those ambiguous generated secondary
cases are skipped and counted. External TSVs are still supported with
`keys<TAB>expected-layout`. Eval output now includes accuracy, confusion counts,
false positives/negatives, failing token lengths, and a bounded failure sample.
The repo also has a curated embedded-secondary seed corpus at
`crates/typeflow-cli/eval/uk-hard.tsv` for punctuation-position letters,
smart-quote pain, and DevOps/security false-positive traps.

Defaults (especially `confidence_margin = 1.0`) are still an educated guess.
Keep running generated eval at useful limits, add hand-curated hard cases for
ambiguous short tokens / code identifiers / mixed-script names, then tune until
accuracy is north of 95%.
See `docs/calibration.md` for the current ambiguity policy.

### Host-driven behavior

The macOS host now enforces `apps.disable_bundle_ids` /
`apps.disable_auto_bundle_ids` and checks Carbon secure event input before
processing key events. Those paths set the FFI host context, clear token state,
and return the event to the client app unchanged. The host
also loads scoring knobs, active secondary language, pack directory, and data
directory from the same config file as the CLI.

Manual convert is hardcoded in the macOS host as a standalone Option
press/release. It is intentionally not configurable. Option+another key cancels
the pending manual convert and stays a normal app shortcut/input sequence.

## Outstanding limitations to be aware of

1. **Once flipped, layout sticks.** When the engine switches to secondary
   mid-token, it stays there for the rest of the token. Keep watching this in
   calibration because a bad mid-token flip is expensive for trust.
2. **Dictionary noise.** OPUS / hermitdave secondary lists may contain Latin proper
   names and English loanwords. Words like "amazon" appear in BOTH dictionaries
   with non-trivial counts, weakening the dict signal on certain tokens.
   Filter step in `typeflow-data` would help.
3. **Secure-field detection is host-signal limited.** The IMK layer checks
   Carbon secure event input and app disable policy before processing keys. If an
   app does not enable secure event input for a sensitive field, Typeflow cannot
   infer that from text content without doing exactly the kind of inspection we
   should avoid.
4. **Sample asymmetry.** EN n-grams come from ~200 MB of OPUS, secondary packs
   may use different corpus sizes. The smoothed floors and overall scale differ between
   languages. Revisit if calibration finds bias.

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
- `docs/release-verification.md` — release artifact checks, universal package
  signing/notarization, and the standalone dylib install-name caveat.
- `macos/Sources/TypeflowKit/Engine.swift` — Swift wrapper already calling the
  staticlib through the C ABI.
- `macos/Sources/TypeflowInputMethod/InputController.swift` — current IMK
  raw keyDown/flagsChanged event path, host context, standalone Option manual
  conversion, and composition action application.
- `crates/typeflow-ffi/include/typeflow.h` — exact ABI to consume.
- `crates/typeflow-ffi/src/lib.rs` — Rust side of the bridge; understand
  `TfEvent` / `TfComposition` / `typeflow_engine_process` before writing Swift.
- `crates/typeflow-ffi/tests/abi_smoke.rs` — public ABI host-buffer simulation
  that Swift should mirror.
- `docs/engine.md#the-composition-protocol` — extra explanation
  for the composition protocol. The invariants doc is the source of truth.

If you're tuning thresholds:

- `docs/engine.md#calibration-how-to-tune` — what to do.
- `docs/calibration.md` — what ambiguous generated cases mean.
- `~/.config/typeflow/config.toml` — where to do it.
- `typeflow config show` — verify what the engine is actually loading.

## Open Questions

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
