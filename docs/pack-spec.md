# Language Pack Specs

`typeflow-data build-pack` creates an installable secondary-language pack from a
TOML spec.

```sh
cargo run --release -p typeflow-data -- build-pack ./uk.toml --out /tmp/uk.typeflow-pack
typeflow pack install /tmp/uk.typeflow-pack
typeflow pack use uk
```

The output directory contains:

```text
pack.toml
ngrams.bin
dict.fst
```

## Spec Format

```toml
id = "uk"
display_name = "Ukrainian"
script = "Cyrillic"
layout = "custom"
alphabet = "абвгґдеєжзиіїйклмнопрстуфхцчшщьюя"

# Local paths are resolved relative to this spec file. HTTP/HTTPS URLs are
# downloaded into the build cache.
corpus = "./uk.txt.gz"
dictionary = "./uk_freq.txt"

# Optional. Omit for unbounded corpus processing.
plaintext_budget_bytes = 200000000

# Optional. Default: 500000
dictionary_top_k = 500000

source_corpus = "OpenSubtitles mono uk"
source_dictionary = "Frequency list"
build_id = "uk-2026-05-07"

[keyboard]
# 33 chars each: a-z, then ` [ ] ; ' , .
unshifted = "..."
shifted = "..."
```

Built-in layouts currently accepted without `[keyboard]`:

- `english-us`
- `russian-jcuken`

The corpus is used for character bigram/trigram probabilities. The dictionary
must be whitespace-separated `word count` lines; the builder lowercases and
filters words through `alphabet`, then builds the FST dictionary.

## Limits

The current engine has 33 physical key positions. Languages needing dead keys,
Option layers, composition, or keys outside `a-z` plus `` ` [ ] ; ' , . `` need
the engine key model expanded first.
