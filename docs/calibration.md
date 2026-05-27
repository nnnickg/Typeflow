# Calibration Policy

The engine is tuned to avoid false positive layout switches. A missed switch can
be followed by manual Option conversion; an unwanted switch mutates the current
token and corrupts subsequent typing state.

## Ambiguous Exact English Tokens

Some secondary-language words render back to physical key strings that are
also exact English dictionary words. Those are not clean failures. They are
ambiguous inputs.

Current policy:

- If the physical key string is an exact English dictionary word, generated
  secondary eval skips that case.
- The engine should keep English for strong exact-English tokens unless a
  future host-level UX provides more context.
- Manual Option conversion is the escape hatch after intentional secondary words
  that collide with common English.

Concrete embedded Ukrainian examples:

| Keys | Secondary rendering | Current result | Policy |
|---|---|---|---|
| `nels` | `туди` | English | acceptable ambiguity |

## Generated Eval

`typeclaw eval --generated N` still checks:

- built-in smoke cases;
- top English dictionary words as English;
- top secondary dictionary words as secondary when their physical key sequence
  is not an exact English dictionary word.

Skipped cases are reported as:

```text
generated: skipped_ambiguous_secondary_exact_en=<count>
```

The count covers exact English collisions, which have no single correct
automatic answer. Secondary words whose physical-key sequence contains
`, . ; ' [ ] \` or a shifted form are not skipped. In non-English layouts those
keys produce real letters (Ukrainian б, ю, ж, є, х, ї, ґ for the embedded
Ukrainian pack), so the engine keeps them in the observed token when the
secondary candidate remains a dictionary prefix or exact word. The same keys
still reset unambiguous English prose punctuation.

That keeps the regression corpus strict where the intended layout is actually
observable from the token, without pretending exact cross-language collisions
have one universally correct automatic answer.

## What Still Needs Human Cases

Generated eval cannot replace curated cases. Add TSV cases for:

- code identifiers and short CLI tokens that must stay English;
- common secondary words whose physical keys are not English words;
- mixed-script names;
- punctuation-position letters;
- manual layout-switch flows.

Current repo seed:

```sh
typeclaw eval crates/typeclaw-cli/eval/uk-hard.tsv
```

That file is intentionally small and tied to the embedded secondary language.
Grow it from real typing failures instead of padding it with synthetic noise.
