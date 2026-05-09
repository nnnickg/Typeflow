# Calibration Policy

The engine is tuned to avoid false positive layout switches. A missed switch
can be corrected with manual conversion; an unwanted switch corrupts normal
typing and is harder to trust.

## Ambiguous Exact English Tokens

Some secondary-language words render back to physical key strings that are
also exact English dictionary words. Those are not clean failures. They are
ambiguous inputs.

Current policy:

- If the physical key string is an exact English dictionary word, generated
  secondary eval skips that case.
- The engine should keep English for strong exact-English tokens unless a
  future host-level UX provides more context.
- Manual conversion is the escape hatch for intentional secondary words that
  collide with common English.

Concrete embedded Ukrainian examples:

| Keys | Secondary rendering | Current result | Policy |
|---|---|---|---|
| `hers` | `руки` | English | acceptable ambiguity |
| `here` | `руку` | English | acceptable ambiguity |
| `nels` | `туди` | English | acceptable ambiguity |
| `herb` | `рукі` | English | not the `руки` key sequence in the embedded layout |

## Generated Eval

`typeflow eval --generated N` still checks:

- built-in smoke cases;
- top English dictionary words as English;
- top secondary dictionary words as secondary when their physical key sequence
  is not an exact English dictionary word.

Skipped ambiguous secondary cases are reported as:

```text
generated: skipped_ambiguous_secondary_exact_en=<count>
```

That keeps the regression corpus strict where the intended layout is actually
observable from the token, without pretending exact English collisions have a
single correct automatic answer.

## What Still Needs Human Cases

Generated eval cannot replace curated cases. Add TSV cases for:

- code identifiers and short CLI tokens that must stay English;
- common secondary words whose physical keys are not English words;
- mixed-script names;
- punctuation-position letters;
- manual-conversion flows once the macOS host exists.

Current repo seed:

```sh
typeflow eval crates/typeflow-cli/eval/uk-hard.tsv
```

That file is intentionally small and tied to the embedded secondary language.
Grow it from real typing failures instead of padding it with synthetic noise.
