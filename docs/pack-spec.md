# Language Pack Specs

`typeflow-data build-pack` creates an installable secondary-language pack from a
TOML spec.

Ukrainian is the embedded secondary language. Use packs for Russian or any
other local secondary language.

```sh
cargo run --release -p typeflow-data -- build-pack docs/examples/ru.toml --out /tmp/ru.typeflow-pack
typeflow pack install /tmp/ru.typeflow-pack
typeflow pack use ru
```

The output directory contains:

```text
pack.toml
ngrams.bin
dict.fst
```

## Spec Format

```toml
id = "ru"
display_name = "Russian"
script = "Cyrillic"
layout = "russian-jcuken"
alphabet = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя"

# Local paths are resolved relative to this spec file. HTTP/HTTPS URLs are
# downloaded into the build cache.
corpus = "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/mono/ru.txt.gz"
dictionary = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/ru/ru_full.txt"

# Optional. Omit for unbounded corpus processing.
plaintext_budget_bytes = 200000000

# Optional. Default: 500000
dictionary_top_k = 500000

source_corpus = "OpenSubtitles mono ru"
source_dictionary = "hermitdave/FrequencyWords 2018 ru"
build_id = "ru-2026-05-07"

[keyboard]
# 34 chars each: a-z, then ` [ ] ; ' , . \
unshifted = "..."
shifted = "..."
```

Built-in layouts currently accepted without `[keyboard]`:

- `english-us`
- `russian-jcuken`
- `ukrainian-jcuken-osx`

The corpus is used for character bigram/trigram probabilities. The dictionary
must be whitespace-separated `word count` lines; the builder lowercases and
filters words through `alphabet`, then builds the FST dictionary.

## Limits

The current engine has 34 physical key positions. Languages needing dead keys,
Option layers, composition, or keys outside `a-z` plus `` ` [ ] ; ' , . \ `` need
the engine key model expanded first.
