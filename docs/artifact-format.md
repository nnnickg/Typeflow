# Artifact And Pack Compatibility

This document defines what version `4` means for embedded artifacts and
external secondary-language packs.

## Embedded Artifacts

The release binary embeds six files from `crates/typeclaw-core/data/`:

```text
en.ngrams.bin
en.dict.fst
en.dict-prefix.bin
uk.ngrams.bin
uk.dict.fst
uk.dict-prefix.bin
```

`*.ngrams.bin` is a TypeClaw n-gram artifact containing `CompiledLanguageData`:

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

`*.dict-prefix.bin` is a TypeClaw dictionary-prefix artifact. It stores two
prebuilt `fst::Map` blobs keyed by UTF-8 prefix bytes: one for capped prefix
frequency sums and one for capped sample counts.

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
dict-prefix.bin
```

`pack.toml` contains:

```toml
format_version = 4
id = "secondary"
display_name = "Secondary"
script = "Cyrillic"
layout = "custom"
punctuation_letter_keys = "[]{};:\"'<>.,"
ngrams = "ngrams.bin"
dict = "dict.fst"
dict_prefix = "dict-prefix.bin"

[keyboard]
unshifted = "..."
shifted = "..."
```

The manifest may also contain `source_corpus`, `source_dictionary`, and
`build_id` metadata. `punctuation_letter_keys` is optional for old packs; when
it is missing, the loader derives the set from the pack keyboard map.

## Artifact Licensing

Embedded artifacts and external packs are generated data, not MIT/Apache source
code. The checked-in embedded artifacts are derived from OPUS OpenSubtitles2018
and `hermitdave/FrequencyWords`; see `../DATA-LICENSE.md` and `../NOTICE.md`
for attribution and redistribution notes.

External packs inherit the licenses and redistribution terms of their corpus and
dictionary inputs. Pack authors should fill `source_corpus` and
`source_dictionary` in `pack.toml` and ship attribution next to the pack.

## Format Version 4

`PACK_FORMAT_VERSION = 4` means:

- `pack.toml` uses the fields above.
- `ngrams`, `dict`, and `dict_prefix` paths are relative paths contained inside
  the pack directory.
- `ngrams.bin` starts with `TFNG0002`, followed by little-endian numeric fields
  and length-prefixed UTF-8 n-gram strings.
- `dict.fst` is an `fst::Map<Vec<u8>>`.
- `dict-prefix.bin` starts with `TFPX0001`, followed by two length-prefixed
  `fst::Map<Vec<u8>>` blobs: prefix → `prefix_sum`, then prefix →
  `prefix_sample`.
- The n-gram artifact's `language_tag` must exactly match manifest `id`.
- `id = "en"` is invalid for secondary packs.
- Keyboard rows, when provided, must contain exactly `PhysicalKey::COUNT`
  characters.
- Each keyboard-row character must be a single non-combining UTF-16 code unit.
  The current keyboard model stores one rendered scalar per physical key, so
  non-BMP and combining output require a future ABI/model expansion.
- `punctuation_letter_keys` contains English-layout punctuation-position
  characters whose physical keys render as secondary-layout letters. The engine
  uses this per-pack set instead of assuming Ukrainian-specific punctuation
  behavior.

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
