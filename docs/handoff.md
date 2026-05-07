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
- `typeflow bench [iterations]` — hot-loop micro-benchmark
- `typeflow model` — print language-pack metadata/fingerprints
- `typeflow pack install/list/use/inspect` — external language-pack workflow.
  Installed packs are directories containing `pack.toml`, `ngrams.bin`, and
  `dict.fst`; the release binary remains standalone.
- `typeflow repl` — interactive raw-mode TTY with live score panel and
  "what you would see in TextEdit" simulated commit buffer
- `typeflow config init/show` — manage `~/.config/typeflow/config.toml`

`--config <path>`, `$TYPEFLOW_CONFIG`, and `~/.config/typeflow/config.toml` are
all honored, in that override precedence.

### FFI (typeflow-ffi) — surface ready, not consumed yet

Header at `crates/typeflow-ffi/include/typeflow.h`. Builds as `cdylib`
(`libtypeflow_ffi.dylib`). The Swift IMK bundle isn't built so this is
not verified inside a real host app yet, but the public ABI has Rust integration
smoke coverage. Release hosts should use
`typeflow_engine_new_embedded()` or
`typeflow_engine_new_embedded_with_config(...)`.
`typeflow_engine_new_from_data_dir(...)` is a dev override for testing rebuilt
model artifacts. `typeflow_engine_new_from_pack_dir(...)` loads embedded English
plus one installed secondary language pack. FFI exposes `TF_EVENT_LITERAL`,
`TF_LAYOUT_SECONDARY`, `_with_config(...)` constructors,
`typeflow_engine_default_config(...)`, modifier-bypass bits, and
`typeflow_engine_set_host_context(...)`.

## What's NOT done

### macOS IMK bundle (`macos/`) — empty placeholder

Nothing here yet beyond a `.gitkeep`. This is the next big milestone. Plan:

1. Create an Xcode project (or a SwiftPM target) under `macos/`.
2. `IMKInputController` subclass that:
   - Receives `keyDown:` / `keyUp:` from `IMKServer`.
   - Translates `event.keyCode` (`kVK_ANSI_*`) to the integer `PhysicalKey`
     index expected by `typeflow_engine_process`.
   - Dispatches via the FFI, applies the returned `TfAction` to the host
     via `client.insertText:replacementRange:`.
3. Bundle `Info.plist` registers as a macOS input source claiming both
   Latin and Cyrillic scripts.
4. `Makefile` target builds `libtypeflow_ffi.dylib`, embeds it in
   `Typeflow.app/Contents/MacOS/`, copies the `.app` to
   `~/Library/Input Methods/`, signals `killall -HUP "Typeflow"`.
5. Manual smoke test: activate the input source in System Settings →
   Keyboard → Input Sources, type `ghsdbn` in TextEdit, see `привіт`.

The non-obvious part: macOS expects each input source to be tied to one script.
The "claims both Latin and Cyrillic" trick is the Punto-style workaround. If
that fails, fall back to registering two paired input sources and switching
between them programmatically (KeyKey-style).

### Regression corpus + calibration

`typeflow eval` still runs the small built-in smoke set. `typeflow eval
--generated [limit-per-layout]` now builds a larger regression corpus from the
loaded dictionaries: top EN words expect English, top secondary words are
rendered back to physical-key strings and expect secondary. External TSVs are
still supported with `keys<TAB>expected-layout`. Eval output now includes
accuracy, confusion counts, false positives/negatives, failing token lengths,
and a bounded failure sample.

Defaults (especially `confidence_margin = 1.0`) are still an educated guess.
Before the IMK bundle ships, run generated eval at useful limits, add
hand-curated hard cases for ambiguous short tokens / code identifiers /
mixed-script names, then tune until accuracy is north of 95%.

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
- `crates/typeflow-ffi/include/typeflow.h` — exact ABI to consume.
- `crates/typeflow-ffi/src/lib.rs` — Rust side of the bridge; understand
  `TfEvent` / `TfAction` / `typeflow_engine_process` before writing Swift.
- `crates/typeflow-ffi/tests/abi_smoke.rs` — public ABI host-buffer simulation
  that Swift should mirror.
- `docs/engine.md#the-action-protocol-host-contract` — extra explanation for
  the action protocol. The invariants doc is the source of truth.

If you're tuning thresholds:

- `docs/engine.md#calibration-how-to-tune` — what to do.
- `~/.config/typeflow/config.toml` — where to do it.
- `typeflow config show` — verify what the engine is actually loading.

## Open questions for the next agent / next session

1. **macOS input source registration trick.** Does a single IMK source claiming
   `kTextServiceInputModeRoman` + Cyrillic actually let us emit both scripts in
   one session, or do we need the two-paired-sources hack? Worth a 1-day spike
   before committing to the architecture.
2. **Embedding strategy.** Should the IMK bundle ship `include_bytes!`-embedded
   data (~10 MB binary) or load from `Bundle.main.resourcePath` as files? File
   loading is cheaper to update; embedding is simpler.
3. **Dictionary expansion.** hermitdave's lists include only attested surface
   forms from OPUS. For rare secondary-language inflections this misses obvious words.
   Worth merging in Hunspell expansions before the regression corpus pass?
4. **Score calibration target.** Are we tuning for max accuracy, or for max
   *user-perceived correctness* (which weighs false-positive switches as worse
   than false-negative no-switches)? Different objectives → different thresholds.
5. **Multi-app config.** Does the user want different thresholds per app
   (e.g. more conservative in code editors)? Schema-wise it's
   `[apps.com.googlecode.iterm2.engine]` — defer until calibration is done.
