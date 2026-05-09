# Data License

Typeflow source code is licensed under `MIT OR Apache-2.0`.

This file covers the generated language-model artifacts and any external packs
built with Typeflow tooling. Those files are data artifacts, not Typeflow source
code, and must not be treated as MIT/Apache-licensed code.

## Checked-In Model Artifacts

The embedded model files are:

- `crates/typeflow-core/data/en.ngrams.bin`
- `crates/typeflow-core/data/uk.ngrams.bin`
- `crates/typeflow-core/data/en.dict.fst`
- `crates/typeflow-core/data/uk.dict.fst`

They are generated statistical artifacts derived from third-party inputs:

- OPUS OpenSubtitles2018 monolingual text data:
  https://opus.nlpl.eu/OpenSubtitles.php
- `hermitdave/FrequencyWords` word-frequency lists:
  https://github.com/hermitdave/FrequencyWords

Redistribution of these artifacts is subject to the third-party data terms of
their inputs. In particular, `hermitdave/FrequencyWords` documents its generated
content as `CC-BY-SA-4.0`.

Treat the checked-in model artifacts as third-party-derived data with
attribution and share-alike obligations, not as MIT/Apache source code. If you
need a MIT/Apache-only distribution, rebuild the artifacts from inputs whose
terms allow that distribution and do not include these checked-in files.

Source attribution and citation details are listed in `NOTICE.md`.

## External Packs

External packs built with `typeflow-data build-pack` inherit the licenses and
redistribution terms of their corpus and dictionary inputs. Pack authors should
fill `source_corpus` and `source_dictionary` in `pack.toml` and document any
required attribution next to the pack.

Typeflow does not validate legal compatibility of pack inputs.
