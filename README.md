# Typeflow

Typeflow is a macOS input method that should make English plus one configurable
secondary keyboard layout disappear while typing. Punto-style auto-detection,
but as a real macOS `InputMethodKit` bundle (not a CGEventTap that backspaces
and retypes).

## Status

Pre-alpha macOS input method. Use it at your own risk: it can rewrite text as
you type. Keep a normal keyboard layout installed as a fallback. The Rust engine
works end-to-end on real data, and `macos/` builds, signs, installs, registers,
and runs a working InputMethodKit app. Manual host testing has verified normal
typing, external pack loading, app exclusions, and standalone Option manual
conversion in real macOS text fields. See [`docs/status.md`](docs/status.md) for
the complete state-of-the-project snapshot.

## Workspace

```text
crates/
├── typeflow-core/   pure Rust engine; scoring, decision, data types
├── typeflow-data/   xtask: downloads OpenSubtitles + hermitdave word lists, builds n-grams + FSTs
├── typeflow-cli/    user-facing CLI: type / stream / repl / predict / pack / config
└── typeflow-ffi/    C ABI bridge for the Swift/IMK bundle
docs/
├── architecture.md  component layout + data flow
├── artifact-format.md  pack/data compatibility policy
├── calibration.md   eval policy and ambiguous-token handling
├── engine.md        scoring math, what the config knobs actually do
├── invariants.md    core/host contract the macOS layer must obey
├── panic-unsafe-audit.md  panic/unsafe audit notes
├── release-verification.md  optimized build checks and packaging caveat
└── status.md        current state, outstanding work, open questions
macos/                Swift staticlib smoke + minimal IMK bundle build
```

## Quick start

### 1. Build the language data

The release binary embeds the language model and runs without external data
files. The raw subtitles are only build-time/training input.

To rebuild the embedded model artifacts, run:

```sh
cargo run --release -p typeflow-data
```

That downloads ~3.7 GB from OpenSubtitles + hermitdave into
`target/typeflow-data-cache/` and produces the small compile-time artifacts:

```text
crates/typeflow-core/data/
├── en.ngrams.bin
├── uk.ngrams.bin
├── en.dict.fst
└── uk.dict.fst
```

The cache is not needed at runtime. Keep it only to avoid re-downloading when
rebuilding the model.

Data-source attribution and generated-artifact license notes are in
[`NOTICE.md`](NOTICE.md) and [`DATA-LICENSE.md`](DATA-LICENSE.md).

### 2. Build and run the CLI

```sh
cargo build --release -p typeflow-cli
./target/release/typeflow --help
```

`target/release/typeflow` is standalone: the model is embedded with
`include_bytes!`. External language packs are optional and installed separately.

Or install it on your PATH:

```sh
cargo install --path crates/typeflow-cli
typeflow --help
```

### 3. Try the engine

```sh
# Per-keystroke trace ending in final score breakdown.
typeflow type ghsdbn        # Ukrainian: привіт
typeflow type typeflow      # English

# Cyrillic input also works (reverse-mapped to physical keys).
typeflow type привіт

# One-shot decision, pipe-friendly.
typeflow predict ghsdbn                  # -> "Ukrainian\tпривіт"
typeflow predict --json ghsdbn           # -> JSON line
typeflow convert type                    # force-convert current token

# Built-in hard-case smoke corpus and generated dictionary regression corpus.
typeflow eval
typeflow eval --generated 500             # 500 EN + 500 secondary dictionary cases
# eval prints accuracy, confusion counts, false positives/negatives, length
# buckets, and a bounded failure sample.
typeflow model

# Real benchmarks live under Cargo's benchmark harness.
cargo bench -p typeflow-core
cargo bench -p typeflow-ffi

# Verify Swift can link the Rust staticlib and call the C ABI.
make -C macos smoke

# Build and ad-hoc sign the minimal IMK app bundle.
make -C macos bundle

# Install, register, and enable the input method for the current user.
make -C macos install-user

# Restart a running copy after reinstall so macOS loads the new binary.
pkill -x Typeflow

# External-pack workflow. The binary itself stays standalone.
cargo run --release -p typeflow-data -- build-pack ./secondary.toml --out /tmp/secondary.typeflow-pack
typeflow pack install /tmp/secondary.typeflow-pack
typeflow pack list
typeflow pack use secondary
typeflow pack inspect secondary

# Stream tokens from stdin.
echo -e "ghsdbn\nhello\nyt" | typeflow stream

# Interactive raw-mode REPL with live score updates.
typeflow repl
```

Pack specs are documented in [`docs/pack-spec.md`](docs/pack-spec.md).

## Configuration

All scoring knobs are exposed via TOML. Generate a fully-commented default:

```sh
typeflow config init        # writes ~/.config/typeflow/config.toml
typeflow config show        # prints effective merged config
typeflow --config /tmp/x.toml type ghsdbn
```

See [`docs/engine.md`](docs/engine.md) for what each config field actually controls.
The macOS input method reads the same config path for engine tuning, active
secondary language packs, and excluded app bundle IDs. `TYPEFLOW_DATA_DIR` and
`TYPEFLOW_PACK_DIR` override TOML in both the CLI and macOS host. Manual
conversion is not configurable in TOML: the macOS host hardcodes standalone
Option press/release.
Option+another key is treated as normal app input.

Example app exclusion config:

```toml
[apps]
exclude_bundle_ids = [
    "com.googlecode.iterm2",
    "dev.zed.Zed",
]
```

## Workspace tests

```sh
cargo test --workspace
```

CI runs fmt, tests, clippy, release tests, release build, and release CLI smoke
on macOS for every push to `main` and every pull request.

## License

Code is licensed under either of:

- Apache License, Version 2.0
- MIT license

at your option.

The embedded language-model artifacts are generated from third-party corpora
and frequency lists. They are data artifacts, not MIT/Apache source code. See
[`DATA-LICENSE.md`](DATA-LICENSE.md) and [`NOTICE.md`](NOTICE.md) for
redistribution terms and attribution.
