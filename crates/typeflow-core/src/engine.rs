use std::sync::Arc;

use crate::data::{LanguageBundle, LanguageModel};
use crate::score::{NgramTotals, has_dictionary_evidence, score_layout, score_layout_with_ngrams};
use crate::{
    Action, Decision, EngineConfig, EngineOutput, HostContext, InputEvent, Layout,
    LayoutCandidates, LetterEvent, MAX_CONFIG_TOKEN_LEN, PhysicalKey, ScoreAnalysis,
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

    pub fn current_token_action(&self) -> Action {
        if self.token.is_empty() {
            return Action::Keep;
        }

        Action::ReplaceToken {
            old_len: self.token.len(),
            replacement: self.candidates.get(self.layout).to_owned(),
            layout: self.layout,
        }
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

    pub fn process(&mut self, event: InputEvent) -> EngineOutput<'_> {
        let (action, decision) = self.step(event);
        let score = self.score_current();
        EngineOutput {
            candidates: &self.candidates,
            score,
            decision,
            action,
        }
    }

    pub fn process_action(&mut self, event: InputEvent) -> Action {
        self.step(event).0
    }

    pub fn force_switch_token(&mut self) -> EngineOutput<'_> {
        if self.token.is_empty() {
            return self.snapshot(Action::Keep, Decision::Keep);
        }

        let target = match self.layout {
            Layout::English => Layout::Secondary,
            Layout::Secondary => Layout::English,
        };
        self.layout = target;

        let replacement = self.candidates.get(target).to_owned();
        let score = self.score_current();
        let decision = Decision::Use(target);
        // The whole token has already been committed by the host (one Commit per
        // letter), so old_len is the full letter count.
        let action = Action::ReplaceToken {
            old_len: self.token.len(),
            replacement,
            layout: target,
        };

        EngineOutput {
            candidates: &self.candidates,
            score,
            decision,
            action,
        }
    }

    pub fn convert_visible_token(&self, token: &str) -> Option<(Layout, String)> {
        if token.is_empty() {
            return None;
        }

        let letters = token
            .chars()
            .map(|character| self.visible_letter_event_from_char(character))
            .collect::<Option<Vec<_>>>()?;
        let english = crate::render_letters_with_bundle(&letters, Layout::English, &self.bundle);
        let secondary =
            crate::render_letters_with_bundle(&letters, Layout::Secondary, &self.bundle);

        let target = if token == english && token != secondary {
            Layout::Secondary
        } else if token == secondary && token != english {
            Layout::English
        } else if token
            .chars()
            .any(|character| character.is_ascii_alphabetic())
        {
            Layout::Secondary
        } else {
            return None;
        };

        let replacement = match target {
            Layout::English => english,
            Layout::Secondary => secondary,
        };

        (replacement != token).then_some((target, replacement))
    }

    pub fn visible_token_suffix<'a>(&self, visible_tail: &'a str) -> Option<&'a str> {
        let mut start = visible_tail.len();
        let mut found = false;

        for (idx, character) in visible_tail.char_indices().rev() {
            if !self.is_visible_token_character(character) {
                break;
            }
            start = idx;
            found = true;
        }

        found.then_some(&visible_tail[start..])
    }

    pub fn convert_visible_tail(&self, visible_tail: &str) -> Option<(Layout, String, usize)> {
        let token = self.visible_token_suffix(visible_tail)?;
        let (layout, replacement) = self.convert_visible_token(token)?;
        Some((layout, replacement, token.chars().count()))
    }

    pub fn replace_visible_prefix_with_key(
        &mut self,
        visible_prefix: &str,
        event: LetterEvent,
        target: Layout,
    ) -> Option<Action> {
        let mut letters = visible_prefix
            .chars()
            .map(|character| self.visible_letter_event_from_char(character))
            .collect::<Option<Vec<_>>>()?;
        let prefix_candidates = crate::render_candidates_with_bundle(&letters, &self.bundle);
        self.token_start_layout = infer_visible_layout(visible_prefix, &prefix_candidates)
            .unwrap_or(self.token_start_layout);

        letters.push(event);
        let candidates = crate::render_candidates_with_bundle(&letters, &self.bundle);
        let replacement = candidates.get(target).to_owned();

        self.token = letters;
        self.candidates = candidates;
        self.recompute_ngrams();
        self.score_cache = None;
        self.bypass_until_boundary = false;
        self.layout = target;

        Some(Action::ReplaceToken {
            old_len: visible_prefix.chars().count(),
            replacement,
            layout: target,
        })
    }

    pub fn replace_visible_tail_with_key(
        &mut self,
        visible_tail: &str,
        event: LetterEvent,
        target: Layout,
    ) -> Option<Action> {
        let visible_prefix = self.visible_token_suffix(visible_tail)?;
        self.replace_visible_prefix_with_key(visible_prefix, event, target)
    }

    /// Single dispatch path shared by `process` and `process_action`. Returns
    /// the host-facing action plus the explanatory decision.
    fn step(&mut self, event: InputEvent) -> (Action, Decision) {
        if self.host_context.bypasses_engine() {
            self.reset_token();
            return (Action::Keep, Decision::Bypass);
        }

        match event {
            InputEvent::Letter(letter) => self.step_letter(letter),
            InputEvent::Literal(character) => self.step_literal(character),
            InputEvent::Backspace => self.step_backspace(),
            InputEvent::EndToken => self.step_end_token(),
            InputEvent::HostBypass => {
                self.reset_token();
                (Action::Keep, Decision::Bypass)
            }
        }
    }

    fn step_letter(&mut self, event: LetterEvent) -> (Action, Decision) {
        let commit_char = self.bundle.render(event, self.layout);
        if self.bypass_until_boundary {
            return (Action::Commit(commit_char), Decision::Bypass);
        }

        if self.token.is_empty() {
            self.token_start_layout = self.layout;
        }

        if self.should_commit_auto_disabled_separator(event) {
            self.reset_token();
            return (Action::Commit(commit_char), Decision::Bypass);
        }
        if self.should_commit_english_separator(event) {
            self.reset_token();
            return (Action::Commit(commit_char), Decision::Keep);
        }

        if self.token.len() >= self.config.max_token_len {
            self.reset_token();
            self.bypass_until_boundary = true;
            return (Action::Commit(commit_char), Decision::Bypass);
        }

        self.token.push(event);
        self.push_candidate_chars(event);

        if self.host_context.automatic_switching_disabled {
            return (Action::Commit(commit_char), Decision::Bypass);
        }

        if self.token.len() < self.config.min_token_len {
            return (Action::Commit(commit_char), Decision::Keep);
        }
        if self.should_bypass_token() {
            return (Action::Commit(commit_char), Decision::Bypass);
        }

        let score = self.score_current();
        let decision = self.decide(&score);
        let action = match decision {
            Decision::Keep | Decision::Bypass => Action::Commit(commit_char),
            Decision::Use(layout) if layout == self.layout => Action::Commit(commit_char),
            Decision::Use(layout) => {
                let replacement = self.candidates.get(layout).to_owned();
                self.layout = layout;
                // The host has only committed the previous letters of this token;
                // the just-pushed letter is implicit in `replacement`. Subtract
                // one so `old_len` matches the host's committed prefix exactly.
                Action::ReplaceToken {
                    old_len: self.token.len() - 1,
                    replacement,
                    layout,
                }
            }
        };
        (action, decision)
    }

    /// A literal (digit, punctuation, separator) terminates the current token
    /// and commits the character itself. Token state is fully cleared so it
    /// stays in sync with the host buffer regardless of how many literals
    /// appear in the input stream.
    fn step_literal(&mut self, character: char) -> (Action, Decision) {
        self.reset_token();
        (Action::Commit(character), Decision::Keep)
    }

    fn step_backspace(&mut self) -> (Action, Decision) {
        if self.bypass_until_boundary {
            return (Action::Keep, Decision::Bypass);
        }

        self.token.pop();
        self.candidates.english.pop();
        self.candidates.secondary.pop();
        self.recompute_ngrams();
        self.score_cache = None;
        if self.host_context.automatic_switching_disabled {
            return (Action::Keep, Decision::Bypass);
        }
        let decision = self.reconcile_layout_after_token_change();
        (Action::Keep, decision)
    }

    fn step_end_token(&mut self) -> (Action, Decision) {
        self.reset_token();
        (Action::ResetToken, Decision::Keep)
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
        if !self.host_context.automatic_switching_disabled || self.layout != Layout::English {
            return false;
        }

        is_english_separator_key_char(self.bundle.render(event, Layout::English))
    }

    fn snapshot(&mut self, action: Action, decision: Decision) -> EngineOutput<'_> {
        let score = self.score_current();

        EngineOutput {
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

    fn is_visible_token_character(&self, character: char) -> bool {
        !is_literal_bypass_char(character)
            && self.visible_letter_event_from_char(character).is_some()
    }

    fn visible_letter_event_from_char(&self, character: char) -> Option<LetterEvent> {
        self.bundle
            .letter_event_from_char(character)
            .or_else(|| host_normalized_letter_event(character))
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

fn infer_visible_layout(token: &str, candidates: &LayoutCandidates) -> Option<Layout> {
    if token == candidates.english && token != candidates.secondary {
        Some(Layout::English)
    } else if token == candidates.secondary && token != candidates.english {
        Some(Layout::Secondary)
    } else {
        None
    }
}

fn host_normalized_letter_event(character: char) -> Option<LetterEvent> {
    match character {
        // Some hosts, notably Slack, rewrite ASCII quotes after insertion.
        // Decode those visible forms as the same physical quote key so fallback
        // replacements still include the complete token.
        '‘' | '’' => Some(LetterEvent {
            physical_key: PhysicalKey::Quote,
            shift: false,
        }),
        '“' | '”' => Some(LetterEvent {
            physical_key: PhysicalKey::Quote,
            shift: true,
        }),
        _ => None,
    }
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
