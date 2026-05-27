use std::sync::Arc;

use crate::data::{LanguageBundle, LanguageModel};
use crate::score::{NgramTotals, has_dictionary_evidence, score_layout, score_layout_with_ngrams};
use crate::{
    Decision, EngineConfig, HostContext, InputEvent, Layout, LayoutCandidates, LetterEvent,
    MAX_CONFIG_TOKEN_LEN, ObservationAction, ObservationOutput, ScoreAnalysis,
};

pub struct Engine {
    config: EngineConfig,
    bundle: Arc<LanguageBundle>,
    token: Vec<LetterEvent>,
    candidates: LayoutCandidates,
    ngrams: LayoutNgrams,
    score_cache: Option<ScoreAnalysis>,
    layout: Layout,
    token_start_layout: Layout,
    bypass_until_boundary: bool,
    host_context: HostContext,
}

#[derive(Clone, Debug, Default)]
struct LayoutNgrams {
    english: RollingNgrams,
    secondary: RollingNgrams,
}

impl LayoutNgrams {
    fn clear(&mut self) {
        self.english.clear();
        self.secondary.clear();
    }

    fn pop(&mut self) {
        self.english.pop();
        self.secondary.pop();
    }
}

#[derive(Clone, Debug, Default)]
struct RollingNgrams {
    raw_bigram: f32,
    raw_trigram: f32,
    normalized: Vec<char>,
    chunk_lens: Vec<usize>,
    bigram_contribs: Vec<f32>,
    trigram_contribs: Vec<f32>,
}

impl RollingNgrams {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn push(&mut self, model: &LanguageModel, character: char) {
        let start_len = self.normalized.len();
        for normalized in character.to_lowercase() {
            self.push_normalized(model, normalized);
        }
        self.chunk_lens.push(self.normalized.len() - start_len);
    }

    fn push_normalized(&mut self, model: &LanguageModel, current: char) {
        let len = self.normalized.len();
        let bigram = if len >= 1 {
            model.bigram_log_prob_for_chars(self.normalized[len - 1], current)
        } else {
            0.0
        };
        let trigram = if len >= 2 {
            model.trigram_log_prob_for_chars(
                self.normalized[len - 2],
                self.normalized[len - 1],
                current,
            )
        } else {
            0.0
        };

        self.raw_bigram += bigram;
        self.raw_trigram += trigram;
        self.normalized.push(current);
        self.bigram_contribs.push(bigram);
        self.trigram_contribs.push(trigram);
    }

    fn pop(&mut self) {
        let Some(count) = self.chunk_lens.pop() else {
            return;
        };
        for _ in 0..count {
            self.normalized.pop();
            if let Some(score) = self.bigram_contribs.pop() {
                self.raw_bigram -= score;
            }
            if let Some(score) = self.trigram_contribs.pop() {
                self.raw_trigram -= score;
            }
        }
    }

    fn totals(&self) -> NgramTotals {
        NgramTotals {
            raw_bigram: self.raw_bigram,
            raw_trigram: self.raw_trigram,
            char_count: self.normalized.len(),
        }
    }
}

impl Engine {
    pub fn new(config: EngineConfig, bundle: LanguageBundle) -> Self {
        Self::with_shared_bundle(config, Arc::new(bundle))
    }

    pub fn with_shared_bundle(config: EngineConfig, bundle: Arc<LanguageBundle>) -> Self {
        let token_capacity = config.max_token_len.min(MAX_CONFIG_TOKEN_LEN);
        Self {
            config,
            bundle,
            token: Vec::with_capacity(token_capacity),
            candidates: LayoutCandidates {
                english: String::with_capacity(token_capacity),
                secondary: String::with_capacity(token_capacity * 4),
            },
            ngrams: LayoutNgrams::default(),
            score_cache: None,
            layout: Layout::English,
            token_start_layout: Layout::English,
            bypass_until_boundary: false,
            host_context: HostContext::default(),
        }
    }

    pub fn current_layout(&self) -> Layout {
        self.layout
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn bundle(&self) -> &LanguageBundle {
        self.bundle.as_ref()
    }

    pub fn token_len(&self) -> usize {
        self.token.len()
    }

    pub fn token_candidates(&self) -> &LayoutCandidates {
        &self.candidates
    }

    pub fn token_score(&mut self) -> ScoreAnalysis {
        self.score_current()
    }

    pub fn reset_token(&mut self) {
        self.token.clear();
        self.candidates.english.clear();
        self.candidates.secondary.clear();
        self.ngrams.clear();
        self.score_cache = None;
        self.bypass_until_boundary = false;
        self.token_start_layout = self.layout;
    }

    pub fn reset_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.token.clear();
        self.candidates.english.clear();
        self.candidates.secondary.clear();
        self.ngrams.clear();
        self.score_cache = None;
        self.bypass_until_boundary = false;
        self.token_start_layout = layout;
    }

    pub fn letter_event_from_char(&self, value: char) -> Option<LetterEvent> {
        self.bundle.letter_event_from_char(value)
    }

    /// Converts a character into the appropriate `InputEvent` for this engine's
    /// loaded language bundle. Characters that don't map to any physical key
    /// position (digits, most ASCII symbols) become `InputEvent::Literal`.
    ///
    /// This is the correct entry point when driving the engine from text input
    /// (CLI, tests, pipes) rather than from physical keycodes (macOS/FFI).
    pub fn input_event_from_char(&self, character: char) -> InputEvent {
        if is_literal_bypass_char(character) {
            return InputEvent::Literal(character);
        }
        self.letter_event_from_char(character)
            .map(InputEvent::Letter)
            .unwrap_or(InputEvent::Literal(character))
    }

    pub fn host_context(&self) -> HostContext {
        self.host_context
    }

    pub fn set_host_context(&mut self, context: HostContext) {
        if self.host_context == context {
            if context.bypasses_engine() {
                self.reset_token();
            }
            return;
        }

        self.host_context = context;
        self.reset_token();
    }

    pub fn observe(&mut self, event: InputEvent) -> ObservationOutput<'_> {
        let (action, decision) = self.step(event);
        self.snapshot(action, decision)
    }

    pub fn force_switch_layout(&mut self) -> ObservationOutput<'_> {
        if self.host_context.bypasses_engine() {
            let action = self.reset_action();
            self.reset_token();
            return self.snapshot(action, Decision::Bypass);
        }

        let target = match self.layout {
            Layout::English => Layout::Secondary,
            Layout::Secondary => Layout::English,
        };
        self.layout = target;
        self.reset_token();
        self.snapshot(
            ObservationAction::SwitchFutureLayout(target),
            Decision::Use(target),
        )
    }

    fn step(&mut self, event: InputEvent) -> (ObservationAction, Decision) {
        if self.host_context.bypasses_engine() {
            let action = self.reset_action();
            self.reset_token();
            return (action, Decision::Bypass);
        }

        match event {
            InputEvent::Letter(letter) => self.step_letter(letter),
            InputEvent::Literal(character) => self.step_literal(character),
            InputEvent::Backspace => self.step_backspace(),
            InputEvent::EndToken => self.step_end_token(),
            InputEvent::HostBypass => {
                let action = self.reset_action();
                self.reset_token();
                (action, Decision::Bypass)
            }
        }
    }

    fn step_letter(&mut self, event: LetterEvent) -> (ObservationAction, Decision) {
        if self.bypass_until_boundary {
            if self.is_english_punctuation_letter_key(event) {
                let action = self.reset_action();
                self.reset_token();
                return (action, Decision::Bypass);
            }
            return (ObservationAction::None, Decision::Bypass);
        }

        if self.token.is_empty() {
            self.token_start_layout = self.layout;
        }

        if self.token.len() >= self.config.max_token_len {
            let action = self.reset_action();
            self.reset_token();
            self.bypass_until_boundary = true;
            return (action, Decision::Bypass);
        }

        let had_token_before_event = !self.token.is_empty();
        self.token.push(event);
        self.push_candidate_chars(event);

        if self.should_reset_on_separator(event) {
            self.reset_token();
            let action = if had_token_before_event {
                ObservationAction::ResetToken
            } else {
                ObservationAction::None
            };
            return (action, Decision::Keep);
        }

        if self.host_context.automatic_switching_disabled {
            return (ObservationAction::None, Decision::Bypass);
        }

        if self.token.len() < self.config.min_token_len {
            return (ObservationAction::None, Decision::Keep);
        }
        if self.should_bypass_token() {
            return (ObservationAction::None, Decision::Bypass);
        }

        let score = self.score_current();
        let decision = self.decide(&score);
        let previous_layout = self.layout;
        match decision {
            Decision::Keep | Decision::Bypass => {}
            Decision::Use(layout) if layout == self.layout => {}
            Decision::Use(layout) => {
                self.layout = layout;
            }
        }
        (self.switch_action(previous_layout), decision)
    }

    fn step_literal(&mut self, _character: char) -> (ObservationAction, Decision) {
        let action = self.reset_action();
        self.reset_token();
        (action, Decision::Keep)
    }

    fn step_backspace(&mut self) -> (ObservationAction, Decision) {
        if self.bypass_until_boundary || self.token.is_empty() {
            return (ObservationAction::None, Decision::Bypass);
        }

        self.token.pop();
        self.candidates.english.pop();
        self.candidates.secondary.pop();
        self.ngrams.pop();
        self.score_cache = None;
        if self.token.is_empty() {
            self.reset_token();
            return (ObservationAction::ResetToken, Decision::Keep);
        }
        if self.host_context.automatic_switching_disabled {
            return (ObservationAction::None, Decision::Bypass);
        }
        let previous_layout = self.layout;
        let decision = self.reconcile_layout_after_token_change();
        (self.switch_action(previous_layout), decision)
    }

    fn step_end_token(&mut self) -> (ObservationAction, Decision) {
        let action = self.reset_action();
        self.reset_token();
        (action, Decision::Keep)
    }

    /// Scores arbitrary candidates against this engine's language bundle and config.
    ///
    /// This does **not** use or populate the internal score cache — the cache
    /// only applies to `self.candidates` via `score_current()`. Use this method
    /// for one-off comparisons; use `token_score()` for the cached hot path.
    pub fn score(&self, candidates: &LayoutCandidates) -> ScoreAnalysis {
        let english = self.bundle.pack(Layout::English);
        let secondary = self.bundle.pack(Layout::Secondary);
        ScoreAnalysis {
            english: score_layout(
                Layout::English,
                &candidates.english,
                &english.model,
                &english.dict,
                &english.dict_index,
                &self.config,
            ),
            secondary: score_layout(
                Layout::Secondary,
                &candidates.secondary,
                &secondary.model,
                &secondary.dict,
                &secondary.dict_index,
                &self.config,
            ),
        }
    }

    fn decide(&self, score: &ScoreAnalysis) -> Decision {
        if self.token.len() < self.config.min_token_len {
            return Decision::Keep;
        }

        if self.should_bypass_token() {
            return Decision::Bypass;
        }

        if self.can_switch_to(Layout::English, score) {
            Decision::Use(Layout::English)
        } else if self.can_switch_to(Layout::Secondary, score) {
            Decision::Use(Layout::Secondary)
        } else {
            Decision::Keep
        }
    }

    fn can_switch_to(&self, layout: Layout, score: &ScoreAnalysis) -> bool {
        let layout_score = match layout {
            Layout::English => score.english,
            Layout::Secondary => score.secondary,
        };
        let margin = score.margin_for(layout);
        let threshold = if has_dictionary_evidence(layout_score) {
            self.config.confidence_margin
        } else {
            self.config.ngram_only_confidence_margin
        };

        margin >= threshold
    }

    fn reconcile_layout_after_token_change(&mut self) -> Decision {
        if self.token.is_empty() {
            self.layout = self.token_start_layout;
            return Decision::Keep;
        }

        let score = self.score_current();
        let decision = self.decide(&score);

        match decision {
            Decision::Use(layout) => self.layout = layout,
            Decision::Keep | Decision::Bypass => self.layout = self.token_start_layout,
        }

        decision
    }

    fn should_bypass_token(&self) -> bool {
        (self.config.disable_on_internal_caps && has_internal_caps(&self.token))
            || is_acronym_like(&self.token)
    }

    fn should_reset_on_separator(&mut self, event: LetterEvent) -> bool {
        if self.layout != Layout::English || !self.is_english_punctuation_letter_key(event) {
            return false;
        }

        !has_dictionary_evidence(self.score_current().secondary)
    }

    fn is_english_punctuation_letter_key(&self, event: LetterEvent) -> bool {
        let english = self.bundle.render(event, Layout::English);
        self.bundle
            .secondary
            .punctuation_letter_keys
            .contains(english)
    }

    fn reset_action(&self) -> ObservationAction {
        if self.token.is_empty() {
            ObservationAction::None
        } else {
            ObservationAction::ResetToken
        }
    }

    fn switch_action(&self, previous_layout: Layout) -> ObservationAction {
        if self.layout == previous_layout {
            ObservationAction::None
        } else {
            ObservationAction::SwitchFutureLayout(self.layout)
        }
    }

    fn snapshot(&mut self, action: ObservationAction, decision: Decision) -> ObservationOutput<'_> {
        let score = self.score_current();

        ObservationOutput {
            candidates: &self.candidates,
            score,
            decision,
            action,
        }
    }

    fn push_candidate_chars(&mut self, event: LetterEvent) {
        let english = self.bundle.render(event, Layout::English);
        let secondary = self.bundle.render(event, Layout::Secondary);
        self.ngrams
            .english
            .push(&self.bundle.pack(Layout::English).model, english);
        self.ngrams
            .secondary
            .push(&self.bundle.pack(Layout::Secondary).model, secondary);
        self.candidates.english.push(english);
        self.candidates.secondary.push(secondary);
        self.score_cache = None;
    }

    fn score_current(&mut self) -> ScoreAnalysis {
        if let Some(score) = self.score_cache {
            return score;
        }

        let english = self.bundle.pack(Layout::English);
        let secondary = self.bundle.pack(Layout::Secondary);
        let score = ScoreAnalysis {
            english: score_layout_with_ngrams(
                Layout::English,
                &self.candidates.english,
                self.ngrams.english.totals(),
                &english.dict,
                &english.dict_index,
                &self.config,
            ),
            secondary: score_layout_with_ngrams(
                Layout::Secondary,
                &self.candidates.secondary,
                self.ngrams.secondary.totals(),
                &secondary.dict,
                &secondary.dict_index,
                &self.config,
            ),
        };
        self.score_cache = Some(score);
        score
    }
}

/// Returns true when the token contains a Shift-modified letter at any position
/// other than the first — i.e., camelCase / PascalCase / acronym-style writing.
/// Capitalized first-letter words like "Hello" return false.
fn has_internal_caps(token: &[LetterEvent]) -> bool {
    token.iter().skip(1).any(|event| event.shift)
}

fn is_acronym_like(token: &[LetterEvent]) -> bool {
    token.len() >= 2 && token.iter().all(|event| event.shift)
}

/// True for characters that are NOT mapped to any physical key position in
/// either keyboard layout — digits and ASCII symbols that don't have a
/// Cyrillic letter on the same key.
///
/// Keyboard-position chars like `,` `.` `;` `'` `[` `]` `\` `` ` `` and their
/// shifted forms `<` `>` `:` `"` `{` `}` `|` `~` are intentionally NOT in this
/// list: in Cyrillic layouts the same physical keys can produce letters. The
/// engine must see them as Letter events so it can score words typed through
/// punctuation-position keys and pick the right side.
pub fn is_literal_bypass_char(character: char) -> bool {
    character.is_ascii_digit()
        || matches!(
            character,
            '!' | '@'
                | '#'
                | '$'
                | '%'
                | '^'
                | '&'
                | '*'
                | '('
                | ')'
                | '_'
                | '-'
                | '+'
                | '='
                | '/'
                | '?'
        )
}
