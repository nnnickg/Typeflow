# Language Pack Specs

`typeflow-data build-pack` creates an installable secondary-language pack from a
TOML spec.

Ukrainian is the embedded secondary language. Use packs for any other local
secondary language.

```sh
cargo run --release -p typeflow-data -- build-pack ./secondary.toml --out /tmp/secondary.typeflow-pack
typeflow pack install /tmp/secondary.typeflow-pack
typeflow pack use secondary
```

The output directory contains:

```text
pack.toml
ngrams.bin
dict.fst
```

## Spec Format

```toml
id = "secondary"
display_name = "Secondary"
script = "Cyrillic"
layout = "custom"
alphabet = "..."

# Local paths are resolved relative to this spec file. HTTP/HTTPS URLs are
# downloaded into the build cache.
corpus = "./secondary.txt.gz"
dictionary = "./secondary_freq.txt"

# Optional. Omit for unbounded corpus processing.
plaintext_budget_bytes = 200000000

# Optional. Default: 500000
dictionary_top_k = 500000

source_corpus = "secondary corpus"
source_dictionary = "secondary dictionary"
build_id = "secondary-2026-05-07"

[keyboard]
# 34 chars each: a-z, then ` [ ] ; ' , . \
# Each char must be one non-combining UTF-16 code unit.
unshifted = "..."
shifted = "..."
```

Built-in layouts currently accepted without `[keyboard]`:

- `english-us`
- `ukrainian-jcuken-osx`

The corpus is used for character bigram/trigram probabilities. The dictionary
must be whitespace-separated `word count` lines; the builder lowercases and
filters words through `alphabet`, then builds the FST dictionary.

## Limits

The current engine has 34 physical key positions and one scalar output per key
state. Languages needing dead keys, Option layers, composition, non-BMP key
outputs, combining marks, or keys outside `a-z` plus `` ` [ ] ; ' , . \ `` need
the engine/ABI key model expanded first.
