use crate::data::{DictLookup, LanguageModel, dict_lookup};
use crate::{EngineConfig, Layout, LayoutScore};

pub(crate) fn score_layout(
    layout: Layout,
    text: &str,
    model: &LanguageModel,
    dict: &fst::Map<Vec<u8>>,
    config: &EngineConfig,
) -> LayoutScore {
    let normalized = text.to_lowercase();
    let score_text = normalized.as_str();

    let raw_bigram = model.score_bigrams(score_text);
    let raw_trigram = model.score_trigrams(score_text);

    let char_count = score_text.chars().count() as f32;
    let (bigram_div, trigram_div) = if config.length_normalize {
        ((char_count - 1.0).max(1.0), (char_count - 2.0).max(1.0))
    } else {
        (1.0, 1.0)
    };

    let bigram = config.bigram_weight * raw_bigram / bigram_div;
    let trigram = config.trigram_weight * raw_trigram / trigram_div;

    let lookup: DictLookup = if score_text.is_empty() {
        DictLookup::default()
    } else {
        dict_lookup(score_text, dict)
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

pub fn has_dictionary_evidence(score: LayoutScore) -> bool {
    score.exact_count > 0 || score.prefix_sum.saturating_sub(score.exact_count) > 0
}
