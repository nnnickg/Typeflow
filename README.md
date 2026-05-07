# Typeflow

Typeflow is a macOS input method that should make English plus one configurable
secondary keyboard layout disappear while typing. Punto-style auto-detection,
but as a real macOS `InputMethodKit` bundle (not a CGEventTap that backspaces
and retypes).

## Status

Pre-alpha. The Rust engine works end-to-end on real data. The macOS Swift/IMK
bundle is **not built yet**. See [`docs/handoff.md`](docs/handoff.md) for the
complete state-of-the-project snapshot.

## Workspace

```text
crates/
├── typeflow-core/   pure Rust engine; scoring, decision, data types
├── typeflow-data/   xtask: downloads OpenSubtitles + hermitdave word lists, builds n-grams + FSTs
├── typeflow-cli/    user-facing CLI: type / stream / repl / predict / pack / config
└── typeflow-ffi/    C ABI bridge for the future Swift/IMK bundle
docs/
├── architecture.md  component layout + data flow
├── engine.md        scoring math, what the config knobs actually do
└── handoff.md       current state, outstanding work, open questions
macos/                placeholder; IMK bundle not yet built
```

## Quick start

### 1. Build the language data

The release binary embeds the language model and runs without external data
files. The raw subtitles are only build-time/training input.

To rebuild the embedded model artifacts, run:

```sh
cargo run --release -p typeflow-data
```

That downloads ~4.3 GB from OpenSubtitles + hermitdave into
`target/typeflow-data-cache/` and produces the small compile-time artifacts:

```text
crates/typeflow-core/data/
├── en.ngrams.bin
├── ru.ngrams.bin
├── en.dict.fst
└── ru.dict.fst
```

The cache is not needed at runtime. Keep it only to avoid re-downloading when
rebuilding the model.

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
typeflow type ghbdtn        # Russian: привет
typeflow type typeflow      # English

# Cyrillic input also works (reverse-mapped to physical keys).
typeflow type привет

# One-shot decision, pipe-friendly.
typeflow predict ghbdtn                  # -> "Russian\tпривет"
typeflow predict --json ghbdtn           # -> JSON line
typeflow convert type                    # force-convert current token

# Built-in hard-case smoke corpus, generated dictionary regression corpus,
# and hot-loop benchmark.
typeflow eval
typeflow eval --generated 500             # 500 EN + 500 secondary dictionary cases
typeflow bench 50000
typeflow model

# External-pack workflow. The binary itself stays standalone.
cargo run --release -p typeflow-data -- build-pack ./language.toml --out /tmp/lang.typeflow-pack
typeflow pack export-ru /tmp/ru.typeflow-pack
typeflow pack install /tmp/ru.typeflow-pack
typeflow pack list
typeflow pack use ru
typeflow pack inspect ru

# Stream tokens from stdin.
echo -e "ghbdtn\nhello\nyt" | typeflow stream

# Interactive raw-mode REPL with live score updates.
typeflow repl
```

Pack specs are documented in [`docs/pack-spec.md`](docs/pack-spec.md).

## Configuration

All scoring knobs are exposed via TOML. Generate a fully-commented default:

```sh
typeflow config init        # writes ~/.config/typeflow/config.toml
typeflow config show        # prints effective merged config
typeflow --config /tmp/x.toml type ghbdtn
```

See [`docs/engine.md`](docs/engine.md) for what each config field actually controls.

## Workspace tests

```sh
cargo test --workspace
```

## License

Licensed under either of:

- Apache License, Version 2.0
- MIT license

at your option.
