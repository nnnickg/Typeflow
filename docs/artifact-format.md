# Artifact And Pack Compatibility

This document defines what version `2` means for embedded artifacts and
external secondary-language packs.

## Embedded Artifacts

The release binary embeds four files from `crates/typeflow-core/data/`:

```text
en.ngrams.bin
en.dict.fst
uk.ngrams.bin
uk.dict.fst
```

`*.ngrams.bin` is a Typeflow n-gram artifact containing `CompiledLanguageData`:

```rust
pub struct CompiledLanguageData {
    pub language_tag: String,
    pub bigrams: Vec<(String, f32)>,
    pub trigrams: Vec<(String, f32)>,
    pub bigram_floor: f32,
    pub trigram_floor: f32,
}
```

`*.dict.fst` is a BurntSushi `fst::Map` keyed by UTF-8 word bytes with `u64`
frequency values.

Embedded artifacts do not have an external manifest. Their compatibility is
compiled into the binary:

- English must deserialize with `language_tag = "en"`.
- Embedded secondary must deserialize with `language_tag = "uk"`.
- Any n-gram/FST parsing failure is startup failure.

## External Pack Layout

External packs are directories with exactly the files referenced by
`pack.toml`. Installed packs are normalized to:

```text
pack.toml
ngrams.bin
dict.fst
```

`pack.toml` contains:

```toml
format_version = 2
id = "secondary"
display_name = "Secondary"
script = "Cyrillic"
layout = "custom"
ngrams = "ngrams.bin"
dict = "dict.fst"

[keyboard]
unshifted = "..."
shifted = "..."
```

The manifest may also contain `source_corpus`, `source_dictionary`, and
`build_id` metadata.

## Format Version 2

`PACK_FORMAT_VERSION = 2` means:

- `pack.toml` uses the fields above.
- `ngrams` and `dict` paths are relative paths contained inside the pack
  directory.
- `ngrams.bin` starts with `TFNG0002`, followed by little-endian numeric fields
  and length-prefixed UTF-8 n-gram strings.
- `dict.fst` is an `fst::Map<Vec<u8>>`.
- The n-gram artifact's `language_tag` must exactly match manifest `id`.
- `id = "en"` is invalid for secondary packs.
- Keyboard rows, when provided, must contain exactly `PhysicalKey::COUNT`
  characters.

The loader rejects any manifest whose `format_version` is not exactly
`PACK_FORMAT_VERSION`.

## When To Bump The Version

Bump `PACK_FORMAT_VERSION` when any existing installed pack would be decoded
incorrectly or with different semantics:

- changing `CompiledLanguageData` fields or serialization format;
- changing dictionary file format;
- changing manifest required fields;
- changing keyboard-row semantics;
- changing `PhysicalKey` count or order for pack keyboard rows;
- changing language-tag matching rules.

Do not bump it for:

- new optional manifest metadata;
- different corpus inputs;
- changed n-gram/dictionary contents generated under the same schema;
- tuning `EngineConfig` defaults.

## Current Compatibility Policy

No backward compatibility shim exists yet. Version mismatch is a hard error.
That is intentional until there is a real installed-pack migration requirement.
