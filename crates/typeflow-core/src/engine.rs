use std::sync::Arc;

use crate::data::{LanguageBundle, LanguageModel};
use crate::score::{NgramTotals, has_dictionary_evidence, score_layout, score_layout_with_ngrams};
use crate::{
    CompositionAction, CompositionOutput, Decision, EngineConfig, HostContext, InputEvent, Layout,
    LayoutCandidates, LetterEvent, MAX_CONFIG_TOKEN_LEN, ScoreAnalysis,
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

#[derive(Clone, Copy, Debug, Default)]
struct LayoutNgrams {
    english: RollingNgrams,
    secondary: RollingNgrams,
}

impl LayoutNgrams {
    fn clear(&mut self) {
        self.english.clear();
        self.secondary.clear();
    }

    fn from_candidates(candidates: &LayoutCandidates, bundle: &LanguageBundle) -> Self {
        Self {
            english: RollingNgrams::from_text(
                &bundle.pack(Layout::English).model,
                &candidates.english,
            ),
            secondary: RollingNgrams::from_text(
                &bundle.pack(Layout::Secondary).model,
                &candidates.secondary,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct RollingNgrams {
    raw_bigram: f32,
    raw_trigram: f32,
    char_count: usize,
    previous_two: char,
    previous_one: char,
}

impl RollingNgrams {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn from_text(model: &LanguageModel, text: &str) -> Self {
        let mut ngrams = Self::default();
        for character in text.chars() {
            ngrams.push(model, character);
        }
        ngrams
    }

    fn push(&mut self, model: &LanguageModel, character: char) {
        for normalized in character.to_lowercase() {
            self.push_normalized(model, normalized);
        }
    }

    fn push_normalized(&mut self, model: &LanguageModel, current: char) {
        self.char_count += 1;
        if self.char_count >= 2 {
            self.raw_bigram += model.bigram_log_prob_for_chars(self.previous_one, current);
        }
        if self.char_count >= 3 {
            self.raw_trigram +=
                model.trigram_log_prob_for_chars(self.previous_two, self.previous_one, current);
        }
        self.previous_two = self.previous_one;
        self.previous_one = current;
    }

    fn totals(self) -> NgramTotals {
        NgramTotals {
            raw_bigram: self.raw_bigram,
            raw_trigram: self.raw_trigram,
            char_count: self.char_count,
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
    /// (CLI, tests, pipes) rather than from physical keycodes (IMK/FFI).
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

    pub fn process(&mut self, event: InputEvent) -> CompositionOutput<'_> {
        let (action, decision) = self.step(event);
        self.snapshot(action, decision)
    }

    pub fn force_switch_token(&mut self) -> CompositionOutput<'_> {
        if self.token.is_empty() {
            return self.snapshot(CompositionAction::Bypass, Decision::Keep);
        }

        let target = match self.layout {
            Layout::English => Layout::Secondary,
            Layout::Secondary => Layout::English,
        };
        self.layout = target;
        self.snapshot(self.render_action(), Decision::Use(target))
    }

    fn step(&mut self, event: InputEvent) -> (CompositionAction, Decision) {
        if self.host_context.bypasses_engine() {
            self.reset_token();
            return (CompositionAction::Bypass, Decision::Bypass);
        }

        match event {
            InputEvent::Letter(letter) => self.step_letter(letter),
            InputEvent::Literal(character) => self.step_literal(character),
            InputEvent::Backspace => self.step_backspace(),
            InputEvent::EndToken => self.step_end_token(),
            InputEvent::HostBypass => {
                if self.token.is_empty() {
                    (CompositionAction::Bypass, Decision::Bypass)
                } else {
                    (self.commit_action(false), Decision::Bypass)
                }
            }
        }
    }

    fn step_letter(&mut self, event: LetterEvent) -> (CompositionAction, Decision) {
        if self.bypass_until_boundary {
            if is_english_separator_key_char(self.bundle.render(event, Layout::English)) {
                self.reset_token();
            }
            return (CompositionAction::Bypass, Decision::Bypass);
        }

        if self.token.is_empty() {
            self.token_start_layout = self.layout;
        }

        if self.should_commit_auto_disabled_separator(event) {
            return (self.commit_action_with(event), Decision::Bypass);
        }
        if self.should_commit_english_separator(event) {
            return (self.commit_action_with(event), Decision::Keep);
        }

        if self.token.len() >= self.config.max_token_len {
            let action = self.commit_action_with(event);
            self.bypass_until_boundary = true;
            return (action, Decision::Bypass);
        }

        self.token.push(event);
        self.push_candidate_chars(event);

        if self.host_context.automatic_switching_disabled {
            return (self.render_action(), Decision::Bypass);
        }

        if self.token.len() < self.config.min_token_len {
            return (self.render_action(), Decision::Keep);
        }
        if self.should_bypass_token() {
            return (self.render_action(), Decision::Bypass);
        }

        let score = self.score_current();
        let decision = self.decide(&score);
        match decision {
            Decision::Keep | Decision::Bypass => {}
            Decision::Use(layout) if layout == self.layout => {}
            Decision::Use(layout) => {
                self.layout = layout;
            }
        }
        (self.render_action(), decision)
    }

    fn step_literal(&mut self, character: char) -> (CompositionAction, Decision) {
        if self.token.is_empty() {
            self.reset_token();
            return (CompositionAction::Bypass, Decision::Keep);
        }
        let mut text = self.composition_text();
        text.push(character);
        self.reset_token();
        (
            CompositionAction::Commit {
                text,
                consume_event: true,
            },
            Decision::Keep,
        )
    }

    fn step_backspace(&mut self) -> (CompositionAction, Decision) {
        if self.bypass_until_boundary || self.token.is_empty() {
            return (CompositionAction::Bypass, Decision::Bypass);
        }

        self.token.pop();
        self.candidates.english.pop();
        self.candidates.secondary.pop();
        self.recompute_ngrams();
        self.score_cache = None;
        if self.token.is_empty() {
            self.reset_token();
            return (
                CompositionAction::Clear {
                    consume_event: true,
                },
                Decision::Keep,
            );
        }
        if self.host_context.automatic_switching_disabled {
            return (self.render_action(), Decision::Bypass);
        }
        let decision = self.reconcile_layout_after_token_change();
        (self.render_action(), decision)
    }

    fn step_end_token(&mut self) -> (CompositionAction, Decision) {
        if self.token.is_empty() {
            self.reset_token();
            return (CompositionAction::Bypass, Decision::Keep);
        }
        (self.commit_action(false), Decision::Keep)
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

    fn should_commit_english_separator(&mut self, event: LetterEvent) -> bool {
        if self.layout != Layout::English || self.token.len() < self.config.min_token_len {
            return false;
        }

        let english = self.bundle.render(event, Layout::English);
        if !is_english_separator_key_char(english) {
            return false;
        }

        if self.should_bypass_token() {
            return true;
        }

        let score = self.score_current();
        self.decide(&score) == Decision::Use(Layout::English)
    }

    fn should_commit_auto_disabled_separator(&self, event: LetterEvent) -> bool {
        if self.token.is_empty()
            || !self.host_context.automatic_switching_disabled
            || self.layout != Layout::English
        {
            return false;
        }

        is_english_separator_key_char(self.bundle.render(event, Layout::English))
    }

    fn composition_text(&self) -> String {
        self.candidates.get(self.layout).to_owned()
    }

    fn render_action(&self) -> CompositionAction {
        CompositionAction::Render {
            text: self.composition_text(),
            layout: self.layout,
        }
    }

    fn commit_action(&mut self, consume_event: bool) -> CompositionAction {
        let text = self.composition_text();
        self.reset_token();
        CompositionAction::Commit {
            text,
            consume_event,
        }
    }

    fn commit_action_with(&mut self, event: LetterEvent) -> CompositionAction {
        let mut text = self.composition_text();
        text.push(self.bundle.render(event, self.layout));
        self.reset_token();
        CompositionAction::Commit {
            text,
            consume_event: true,
        }
    }

    fn snapshot(&mut self, action: CompositionAction, decision: Decision) -> CompositionOutput<'_> {
        let score = self.score_current();

        CompositionOutput {
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

    fn recompute_ngrams(&mut self) {
        self.ngrams = LayoutNgrams::from_candidates(&self.candidates, &self.bundle);
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

fn is_english_separator_key_char(character: char) -> bool {
    matches!(
        character,
        '`' | '['
            | ']'
            | '\\'
            | ';'
            | '\''
            | ','
            | '.'
            | '~'
            | '{'
            | '}'
            | '|'
            | ':'
            | '"'
            | '<'
            | '>'
    )
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
