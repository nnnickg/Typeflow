# TypeClaw

![TypeClaw converts wrong-layout typing into the intended word](docs/assets/typeclaw-demo.gif)

[![Download TypeClaw for macOS](https://img.shields.io/badge/download-macOS%20app-0A7AFF?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/nnnickg/TypeClaw/releases/latest/download/TypeClaw-macos-universal.zip)

TypeClaw is local-only: it does not send keystrokes, text, telemetry, or crash
reports anywhere. The macOS app is distributed for user-trust installation with
ad-hoc signing, not Apple Developer notarization, so macOS will require the
normal manual trust step on first launch.

TypeClaw is a macOS background agent that observes typing and switches between
English plus one configurable secondary keyboard layout. It does not render
inline composition or become the active text compositor; when Rust decides a
token was typed in the wrong layout, the agent replaces that token once and
switches the real macOS input source for future keys.

Ukrainian is the built-in default secondary language; other secondary languages
are loaded from local packs.

```sh
typeclaw --version
typeclaw predict ghsdbn
```

## Who this is for

Bilingual and polyglot macOS users who constantly switch keyboard layouts and
want wrong-layout words corrected locally without a cloud service in the path.

## Status

Public alpha macOS observer agent. Keep normal English and secondary keyboard
layouts installed; TypeClaw switches those real system sources for future keys
and replaces decided tokens with synthetic selection plus Unicode events. The
Rust engine works end-to-end on real data, and `macos/` builds and signs the
agent app. See
[`docs/status.md`](docs/status.md) for the complete
state-of-the-project snapshot.

## Workspace

```text
crates/
├── typeclaw-core/   pure Rust engine; scoring, decision, data types
├── typeclaw-host-config/  TOML/env/app-policy resolution for CLI + macOS host
├── typeclaw-data/   xtask: downloads OpenSubtitles + hermitdave word lists, builds n-grams + FSTs
├── typeclaw-cli/    user-facing CLI: type / stream / repl / predict / pack / config
└── typeclaw-ffi/    C ABI bridge for the Swift macOS agent
docs/
├── architecture.md  component layout + data flow
├── artifact-format.md  pack/data compatibility policy
├── calibration.md   eval policy and ambiguous-token handling
├── engine.md        scoring math, what the config knobs actually do
├── invariants.md    core/host contract the macOS layer must obey
├── operator-runbook.md  logs, config checks, permission checks
├── panic-unsafe-audit.md  panic/unsafe audit notes
├── privacy.md       what Input Monitoring/Accessibility data is read and stored
├── release-verification.md  optimized build checks and packaging caveat
└── status.md        current state, outstanding work, open questions
macos/                Swift staticlib smoke + macOS agent bundle build
```

## Quick start

### 1. Build the language data

The release binary embeds the language model and runs without external data
files. The raw subtitles are only build-time/training input.

To rebuild the embedded model artifacts, run:

```sh
cargo run --release -p typeclaw-data
```

That downloads ~3.7 GB from OpenSubtitles + hermitdave into
`target/typeclaw-data-cache/` and produces the small compile-time artifacts:

```text
crates/typeclaw-core/data/
├── en.ngrams.bin
├── uk.ngrams.bin
├── en.dict.fst
├── uk.dict.fst
├── en.dict-prefix.bin
└── uk.dict-prefix.bin
```

The cache is not needed at runtime. Keep it only to avoid re-downloading when
rebuilding the model.

Data-source attribution and generated-artifact license notes are in
[`NOTICE.md`](NOTICE.md) and [`DATA-LICENSE.md`](DATA-LICENSE.md).

### 2. Build and run the CLI

```sh
cargo build --release -p typeclaw-cli
./target/release/typeclaw --help
```

`target/release/typeclaw` is standalone: the model is embedded with
`include_bytes!`. External language packs are optional and installed separately.

Or install it on your PATH:

```sh
cargo install --path crates/typeclaw-cli
typeclaw --help
```

### 3. Try the engine

```sh
# Per-keystroke trace ending in final score breakdown.
typeclaw type ghsdbn        # Ukrainian: привіт
typeclaw type typeclaw      # English

# Cyrillic input also works (reverse-mapped to physical keys).
typeclaw type привіт

# One-shot decision, pipe-friendly.
typeclaw predict ghsdbn                  # -> "Ukrainian\tпривіт"
typeclaw predict --json ghsdbn           # -> JSON line

# Built-in hard-case smoke corpus and generated dictionary regression corpus.
typeclaw eval
typeclaw eval --generated 500             # 500 EN + 500 secondary dictionary cases
# eval prints accuracy, confusion counts, false positives/negatives, length
# buckets, and a bounded failure sample.
typeclaw model

# Real benchmarks live under Cargo's benchmark harness.
cargo bench -p typeclaw-core
cargo bench -p typeclaw-ffi

# Compile fuzz harnesses for artifact and FFI abuse testing.
cargo build --manifest-path fuzz/Cargo.toml --bins --locked

# Verify Swift can link the Rust staticlib and call the C ABI.
make -C macos smoke

# Verify the SwiftPM library/executable targets.
make -C macos swift-package

# Verify the checked-in C header matches the Rust FFI declarations.
cbindgen --quiet --config cbindgen.toml --crate typeclaw-ffi --output crates/typeclaw-ffi/include/typeclaw.h --verify

# Build and ad-hoc sign the macOS agent app bundle.
make -C macos bundle

# Build a universal macOS release zip with ad-hoc signing.
make -C macos release-universal

# Install, start, and register the agent as a login item for the current user.
make -C macos install-user

# On first launch, approve both prompts:
# - Accessibility
# - Input Monitoring

# External-pack workflow. The binary itself stays standalone.
cargo run --release -p typeclaw-data -- build-pack ./secondary.toml --out /tmp/secondary.typeclaw-pack
typeclaw pack install /tmp/secondary.typeclaw-pack
typeclaw pack list
typeclaw pack use secondary
typeclaw pack inspect secondary

# Generate shell completions.
typeclaw completions zsh > /tmp/typeclaw.zsh
typeclaw completions bash > /tmp/typeclaw.bash

# Stream tokens from stdin.
echo -e "ghsdbn\nhello\nyt" | typeclaw stream

# Interactive raw-mode REPL with live score updates.
typeclaw repl
```

Pack specs are documented in [`docs/pack-spec.md`](docs/pack-spec.md).

## Configuration

All scoring knobs are exposed via TOML. Generate a fully-commented default:

```sh
typeclaw config init        # writes ~/.config/typeclaw/config.toml
typeclaw config show        # prints effective merged config
typeclaw --config /tmp/x.toml type ghsdbn
```

See [`docs/engine.md`](docs/engine.md) for what each config field actually controls.
The macOS agent reads the same config path for engine tuning, active secondary
language packs, excluded app bundle IDs, and optional real macOS input-source
IDs. `TYPECLAW_DATA_DIR` and `TYPECLAW_PACK_DIR` override TOML in both the CLI
and macOS host. Manual switching is not configurable in TOML: the macOS host
hardcodes standalone Option press/release.
Option+another key is treated as normal app input.

Privacy and operations docs:

- [`docs/privacy.md`](docs/privacy.md)
- [`docs/operator-runbook.md`](docs/operator-runbook.md)

Example app exclusion config:

```toml
[apps]
# Fully disabled: pass-through observation AND manual Option-switch are skipped.
disable_bundle_ids = [
    "com.1password.1password",
    "dev.zed.Zed",
]

# Auto-disabled: automatic layout switching is skipped, manual Option-switch
# still works in normal non-secure fields.
disable_auto_bundle_ids = [
    "com.microsoft.VSCode",
]
```

Terminal bundles and embedded terminal panes are auto-detected and behave like
`disable_bundle_ids` without needing to be listed (see
[`docs/host-test-matrix.md`](docs/host-test-matrix.md)). The legacy key
`exclude_bundle_ids` is still accepted and maps to `disable_auto_bundle_ids`.

## Workspace tests

```sh
cargo test --workspace
```

CI runs macOS Rust checks, fuzz target builds, dependency security checks, FFI
header verification, Swift staticlib and SwiftPM builds, the macOS agent bundle
build, and release CLI smoke for every push to `main` and every pull request.

## License

Code is licensed under either of:

- Apache License, Version 2.0
- MIT license

at your option.

The embedded language-model artifacts are generated from third-party corpora
and frequency lists. They are data artifacts, not MIT/Apache source code. See
[`DATA-LICENSE.md`](DATA-LICENSE.md) and [`NOTICE.md`](NOTICE.md) for
redistribution terms and attribution.
