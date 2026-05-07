# Engine

How the engine actually decides which layout you meant.

## The classification problem

When the user types `g`, `h`, `s`, `d`, `b`, `n` on a US keyboard layout, they
might have meant:

- English: `ghsdbn` (gibberish)
- Secondary layout, Ukrainian by default: `привіт` (hello)

The ANSI key positions are unambiguous (G H S D B N). The question is which
language model better explains the resulting text. We score both renderings
and pick the higher one if the margin is convincing enough.

## Bigrams and trigrams

A **bigram** is two consecutive characters, e.g. `пр`, `ри`, `ив`, `ві`, `іт`
in `привіт`. A **trigram** is three: `при`, `рив`, `иві`, `віт`.

We precompute the log-probability of every observed bigram and trigram in a
~200 MB sample of English OpenSubtitles and a secondary-language corpus
(Ukrainian by default; other languages via external packs). For a candidate token,
we sum the log-probability of each of its bigrams (and trigrams) under each
language's model. Higher = "more like this language."

Why both? Bigrams are stable but coarse — `er` is common in many languages.
Trigrams are sharper but only kick in once the token has 3+ chars. Combining
them catches both short and long tokens.

Unseen n-grams (e.g., `ьъ` in well-formed Cyrillic text) get a smoothed floor
log-probability. This is the implicit "impossible bigram penalty": EN-typed-as-RU
text generates lots of unseen secondary-layout n-grams, dragging that score way down.

## Dictionary signal

We also keep an FST per language — a BurntSushi `fst::Map<word, count>` built
from the top 500K most-frequent words in each hermitdave list. Two bonuses:

- **Exact match**: rendered token is itself a dictionary word →
  `dict_exact_weight + log10(count + 1)`.
- **Prefix match**: rendered token is a prefix of N other dictionary words →
  `dict_prefix_weight + log10(prefix_sum + 1)`.

Both can fire; their sum is added to the n-gram score. The prefix sum
subtracts the exact count to avoid double-counting.

## The full scoring formula

For each layout L ∈ {English, Secondary}, given the token rendered in L:

```
bigram_div  = max(1, char_count - 1)        if length_normalize else 1
trigram_div = max(1, char_count - 2)        if length_normalize else 1

score_L = bigram_weight  * sum_log_p(bigrams)  / bigram_div
        + trigram_weight * sum_log_p(trigrams) / trigram_div
        + dict_exact_bonus(L)
        + dict_prefix_bonus(L)
```

Then:

```
margin = score_English - score_Secondary
```

If `|margin| >= confidence_margin`, switch to the winning layout (otherwise
keep the current one). Subject to refusal gates *before* this margin check:

1. `token.len() < min_token_len` → keep, don't decide yet.
2. `disable_on_internal_caps` and the token has shift on any non-first letter
   (camelCase / PascalCase) → keep, don't decide.
3. The whole token is shifted (acronym-like, e.g. `HTTP`) → keep, don't decide.

Digits, punctuation, and identifier separators arrive as `InputEvent::Literal`
and terminate the current token — they don't reach the margin check at all
(see "Token boundaries" below).

There is one extra false-positive guard: if the winning candidate has no exact
or prefix dictionary evidence, it must clear `ngram_only_confidence_margin`
instead of `confidence_margin`. This blocks corrections where both candidates
are garbage and one is merely less bad, e.g. `http` → `реез`.

## Config knobs and what they do

All exposed via `~/.config/typeflow/config.toml`. Run `typeflow config init` to
generate the file with full inline comments. Each field:

### `min_token_len` *(default: 4)*

Minimum number of letters before the engine will decide *anything*. Below this,
all letters commit in the current layout and `Decision::Keep` is returned. Lower =
faster reaction but more false positives on short ambiguous prefixes (`не` vs
`yt`, `при` vs `ghb`). Higher = more cautious, sluggish-feeling.

### `confidence_margin` *(default: 1.0)*

Required score margin (in log10-probability units, per character because of
length normalization) before the engine actually flips. With normalization on,
1.0 means "the winning language is on average 10x more likely *per bigram*."

If you turn `length_normalize` off, you'll need to multiply this by the typical
token length (~5 chars) to get equivalent behavior.

Lower → flips more easily (false-positives on ambiguous tokens).
Higher → flips less often (sluggish; wrong-layout text persists).

### `dict_exact_weight` *(default: 5.0)*

Base bonus for an exact dictionary match. Stacks on top of `log10(freq + 1)`
so a common word like `the` gets a much bigger bonus than a rare word like
`xenophobic`. Increase if dictionary matches feel underweighted vs n-grams;
decrease if rare-but-real words flip too eagerly.

### `dict_prefix_weight` *(default: 2.0)*

Same idea for prefix matches. Smaller default than `dict_exact_weight` because
prefix evidence is weaker than an exact match. Increase if the engine is
sluggish on partial words.

### `ngram_only_confidence_margin` *(default: 3.0)*

Required score margin when the winning candidate has no dictionary exact/prefix
evidence. This is deliberately stricter than `confidence_margin`; false-positive
switches on technical tokens are more damaging than a delayed switch on a rare
word.

### `bigram_weight` *(default: 1.0)*

Multiplier on the bigram log-probability sum. Bigrams are coarse but stable.
Tuning up = short tokens flip more aggressively. Tuning down = long tokens get
their say first. Useful if calibration shows the engine is too eager on 4-char
tokens.

### `trigram_weight` *(default: 1.0)*

Same idea for trigrams. Trigrams are more discriminating but need 3+ chars.
Tuning up = sharper decisions on 5+ char tokens. Tuning down = bigrams dominate.

If you only want one signal, set the other's weight to 0 — the engine handles
that gracefully.

### `length_normalize` *(default: true)*

Divide the n-gram sums by the token length before adding to the total. Without
this, the *threshold* is meaningfully length-dependent: short tokens have small
absolute n-gram sums (easy to flip), long tokens have huge absolute sums (hard).
With normalization, the threshold is in per-bigram units and roughly stable
across token lengths. Strongly recommend leaving on.

### `disable_on_internal_caps` *(default: true)*

If the token has Shift active on any letter past the first, refuse to switch.
This catches camelCase / PascalCase identifiers without blocking properly
capitalized words like `Hello` or `Привіт`.

## Token boundaries

Anything that isn't a language letter (digits, punctuation, separators) is sent
to the engine as `InputEvent::Literal(char)`. The engine treats a literal as a
hard token boundary: it clears the current token, emits `Action::Commit(c)`,
and starts the next token fresh. This keeps `engine.token` and the host's
committed buffer perfectly in sync regardless of how many literals appear in
the input stream — backspace through a literal works without desync.

## Calibration: how to tune

The defaults are an educated guess, not a calibrated value. The right way to
tune is against a regression corpus:

1. Start with `typeflow eval --generated <N>`. This derives labeled cases from
   the loaded EN and secondary dictionaries and renders secondary words back to
   physical key sequences.
2. Add external TSV hard cases where needed. TSV format is
   `keys<TAB>expected-layout`.
3. Run `typeflow eval <cases.tsv>` or `typeflow eval --generated <N>`. The
   report includes accuracy, confusion counts, false positives/negatives,
   failure buckets by token length, and a bounded failure sample.
4. Adjust one knob at a time, re-run, see whether accuracy moves.

The generated corpus is intentionally stricter than the old smoke set. For
example, secondary words whose physical keys form valid English words expose
real false negatives; those belong in the calibration report, not hidden from
the test data. Use `typeflow repl` for interactive inspection of any failure.

## The action protocol (host contract)

`docs/invariants.md` is the source of truth for this contract. This section is
the explanatory version.

The engine returns one of four `Action` variants:

| Action | Host should... |
|---|---|
| `Keep` | Nothing. Engine processed a backspace, modifier-only event, or just synced state. |
| `Commit(char)` | Insert the char at the cursor. Token continues. |
| `ReplaceToken { old_len, replacement, layout }` | Delete the trailing `old_len` chars and insert `replacement`. Engine just flipped layouts. |
| `ResetToken` | Token boundary (e.g. space). Engine cleared its buffer. Host inserts the boundary char itself. |

For `ReplaceToken`: `old_len` is exactly the number of chars in the host buffer
that belong to this token at the moment the action is emitted. Two cases:

- **Mid-stream flip** (engine just decided to switch on the latest letter):
  the host has committed `token.len() - 1` chars (one `Commit` per previous
  letter); the new letter is implicit in `replacement`. So `old_len = token.len() - 1`
  and `replacement.chars().count() = token.len()` — net delta of +1 character.
- **Force-switch** (`engine.force_switch_token()`): the host has already
  committed every letter, so `old_len = token.len()` and
  `replacement.chars().count() = token.len()` — net delta of 0.

In both cases `old_len` matches the host's committed prefix exactly, which
maps directly to AppKit's `client.insertText(_:replacementRange:)`:

```swift
let range = NSRange(location: cursor - oldLen, length: oldLen)
client.insertText(replacement, replacementRange: range)
```

AppKit handles the diff atomically, which means undo (Cmd-Z) Just Works in the
focused app.

If the user backspaces inside the current token, the engine re-evaluates the
shortened token. If the shortened token no longer justifies a switch, layout
state rolls back to the layout that was active at token start.
