use crate::data::LanguageBundle;
use crate::score::{has_dictionary_evidence, score_layout};
use crate::{
    Action, Decision, EngineConfig, EngineOutput, HostContext, InputEvent, Layout,
    LayoutCandidates, LetterEvent, ScoreAnalysis,
};

pub struct Engine {
    config: EngineConfig,
    bundle: LanguageBundle,
    token: Vec<LetterEvent>,
    candidates: LayoutCandidates,
    score_cache: Option<ScoreAnalysis>,
    layout: Layout,
    token_start_layout: Layout,
    host_context: HostContext,
}

impl Engine {
    pub fn new(config: EngineConfig, bundle: LanguageBundle) -> Self {
        Self {
            config,
            bundle,
            token: Vec::new(),
            candidates: LayoutCandidates::default(),
            score_cache: None,
            layout: Layout::English,
            token_start_layout: Layout::English,
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
        &self.bundle
    }

    pub fn token_len(&self) -> usize {
        self.token.len()
    }

    pub fn token_candidates(&self) -> LayoutCandidates {
        self.candidates.clone()
    }

    pub fn token_score(&mut self) -> ScoreAnalysis {
        self.score_current()
    }

    pub fn reset_token(&mut self) {
        self.token.clear();
        self.candidates.english.clear();
        self.candidates.secondary.clear();
        self.score_cache = None;
        self.token_start_layout = self.layout;
    }

    pub fn reset_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.token.clear();
        self.candidates.english.clear();
        self.candidates.secondary.clear();
        self.score_cache = None;
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
        self.host_context = context;
        if context.bypasses_engine() {
            self.reset_token();
        }
    }

    pub fn process(&mut self, event: InputEvent) -> EngineOutput {
        let (action, decision) = self.step(event);
        let candidates = self.candidates.clone();
        let score = self.score_current();
        EngineOutput {
            candidates,
            score,
            decision,
            action,
        }
    }

    pub fn process_action(&mut self, event: InputEvent) -> Action {
        self.step(event).0
    }

    pub fn force_switch_token(&mut self) -> EngineOutput {
        if self.token.is_empty() {
            return self.snapshot(Action::Keep, Decision::Keep);
        }

        let target = match self.layout {
            Layout::English => Layout::Secondary,
            Layout::Secondary => Layout::English,
        };
        self.layout = target;

        let candidates = self.candidates.clone();
        let replacement = candidates.get(target).to_owned();
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
            candidates,
            score,
            decision,
            action,
        }
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
        if self.token.is_empty() {
            self.token_start_layout = self.layout;
        }

        let commit_char = self.bundle.render(event, self.layout);
        self.token.push(event);
        self.push_candidate_chars(event);

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
        self.token.pop();
        self.candidates.english.pop();
        self.candidates.secondary.pop();
        self.score_cache = None;
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
        ScoreAnalysis {
            english: score_layout(
                Layout::English,
                &candidates.english,
                &self.bundle.pack(Layout::English).model,
                &self.bundle.pack(Layout::English).dict,
                &self.config,
            ),
            secondary: score_layout(
                Layout::Secondary,
                &candidates.secondary,
                &self.bundle.pack(Layout::Secondary).model,
                &self.bundle.pack(Layout::Secondary).dict,
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

    fn snapshot(&mut self, action: Action, decision: Decision) -> EngineOutput {
        let candidates = self.candidates.clone();
        let score = self.score_current();

        EngineOutput {
            candidates,
            score,
            decision,
            action,
        }
    }

    fn push_candidate_chars(&mut self, event: LetterEvent) {
        self.candidates
            .english
            .push(self.bundle.render(event, Layout::English));
        self.candidates
            .secondary
            .push(self.bundle.render(event, Layout::Secondary));
        self.score_cache = None;
    }

    fn score_current(&mut self) -> ScoreAnalysis {
        if let Some(score) = self.score_cache {
            return score;
        }

        let score = self.score(&self.candidates);
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
