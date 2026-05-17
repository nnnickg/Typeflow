use std::borrow::Cow;

use crate::data::{DictLookup, DictionaryIndex, LanguageModel};
use crate::{EngineConfig, Layout, LayoutScore};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NgramTotals {
    pub raw_bigram: f32,
    pub raw_trigram: f32,
    pub char_count: usize,
}

pub(crate) fn score_layout(
    layout: Layout,
    text: &str,
    model: &LanguageModel,
    dict: &fst::Map<Cow<'static, [u8]>>,
    dict_index: &DictionaryIndex,
    config: &EngineConfig,
) -> LayoutScore {
    let normalized = lowercase_if_needed(text);
    let score_text = normalized.as_ref();

    let (raw_bigram, raw_trigram, char_count) = model.score_ngrams(score_text);
    score_layout_from_ngrams(
        layout,
        score_text,
        NgramTotals {
            raw_bigram,
            raw_trigram,
            char_count,
        },
        dict,
        dict_index,
        config,
    )
}

pub(crate) fn score_layout_from_ngrams(
    layout: Layout,
    text: &str,
    ngrams: NgramTotals,
    dict: &fst::Map<Cow<'static, [u8]>>,
    dict_index: &DictionaryIndex,
    config: &EngineConfig,
) -> LayoutScore {
    let char_count = ngrams.char_count as f32;
    let (bigram_div, trigram_div) = if config.length_normalize {
        ((char_count - 1.0).max(1.0), (char_count - 2.0).max(1.0))
    } else {
        (1.0, 1.0)
    };

    let bigram = config.bigram_weight * ngrams.raw_bigram / bigram_div;
    let trigram = config.trigram_weight * ngrams.raw_trigram / trigram_div;

    let lookup: DictLookup = if text.is_empty() {
        DictLookup::default()
    } else {
        dict_index.lookup(text, dict)
    };

    let dict_exact_bonus = if lookup.exact_count > 0 {
        config.dict_exact_weight + (lookup.exact_count as f32 + 1.0).log10()
    } else {
        0.0
    };

    // Prefix bonus rewards the *additional* signal beyond an exact match: how many
    // OTHER words start with this token. Subtracting the exact count avoids
    // double-counting when both bonuses fire.
    let extra_prefix_sum = lookup.prefix_sum.saturating_sub(lookup.exact_count);
    let dict_prefix_bonus = if extra_prefix_sum > 0 {
        config.dict_prefix_weight + (extra_prefix_sum as f32 + 1.0).log10()
    } else {
        0.0
    };

    let total = bigram + trigram + dict_exact_bonus + dict_prefix_bonus;

    LayoutScore {
        layout,
        total,
        bigram,
        trigram,
        dict_exact_bonus,
        dict_prefix_bonus,
        exact_count: lookup.exact_count,
        prefix_sum: lookup.prefix_sum,
    }
}

pub(crate) fn score_layout_with_ngrams(
    layout: Layout,
    text: &str,
    ngrams: NgramTotals,
    dict: &fst::Map<Cow<'static, [u8]>>,
    dict_index: &DictionaryIndex,
    config: &EngineConfig,
) -> LayoutScore {
    let normalized = lowercase_if_needed(text);
    score_layout_from_ngrams(
        layout,
        normalized.as_ref(),
        ngrams,
        dict,
        dict_index,
        config,
    )
}

pub fn has_dictionary_evidence(score: LayoutScore) -> bool {
    score.exact_count > 0 || score.prefix_sum.saturating_sub(score.exact_count) > 0
}

fn lowercase_if_needed(text: &str) -> Cow<'_, str> {
    let mut output: Option<String> = None;

    let mut resume_from = 0;
    for (idx, character) in text.char_indices() {
        let mut lowercase = character.to_lowercase();
        let Some(first) = lowercase.next() else {
            continue;
        };
        // `to_lowercase` is stable for a given char, so we can peek the
        // expansion by buffering one element ahead — no need to rebuild the
        // iterator with `.skip(1)` later.
        let second = lowercase.next();
        if first == character && second.is_none() {
            continue;
        }

        let mut normalized = String::with_capacity(text.len());
        normalized.push_str(&text[..idx]);
        normalized.push(first);
        if let Some(second) = second {
            normalized.push(second);
            normalized.extend(lowercase);
        }
        resume_from = idx + character.len_utf8();
        output = Some(normalized);
        break;
    }

    let Some(mut normalized) = output else {
        return Cow::Borrowed(text);
    };

    for character in text[resume_from..].chars() {
        normalized.extend(character.to_lowercase());
    }

    Cow::Owned(normalized)
}
