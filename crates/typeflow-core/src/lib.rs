pub mod data;

use crate::data::{DictLookup, LanguageBundle, LanguageModel, dict_lookup};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layout {
    English,
    /// The configurable non-English side of the active language pair.
    Secondary,
}

impl Layout {
    pub const SECONDARY: Self = Self::Secondary;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum PhysicalKey {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Grave,
    LBracket,
    RBracket,
    Semicolon,
    Quote,
    Comma,
    Period,
}

impl PhysicalKey {
    pub const COUNT: usize = 33;

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::A),
            1 => Some(Self::B),
            2 => Some(Self::C),
            3 => Some(Self::D),
            4 => Some(Self::E),
            5 => Some(Self::F),
            6 => Some(Self::G),
            7 => Some(Self::H),
            8 => Some(Self::I),
            9 => Some(Self::J),
            10 => Some(Self::K),
            11 => Some(Self::L),
            12 => Some(Self::M),
            13 => Some(Self::N),
            14 => Some(Self::O),
            15 => Some(Self::P),
            16 => Some(Self::Q),
            17 => Some(Self::R),
            18 => Some(Self::S),
            19 => Some(Self::T),
            20 => Some(Self::U),
            21 => Some(Self::V),
            22 => Some(Self::W),
            23 => Some(Self::X),
            24 => Some(Self::Y),
            25 => Some(Self::Z),
            26 => Some(Self::Grave),
            27 => Some(Self::LBracket),
            28 => Some(Self::RBracket),
            29 => Some(Self::Semicolon),
            30 => Some(Self::Quote),
            31 => Some(Self::Comma),
            32 => Some(Self::Period),
            _ => None,
        }
    }

    /// Maps an English-US layout character (Latin letter or punctuation position)
    /// back to the underlying physical key. Case-insensitive for Latin letters.
    ///
    /// This only covers the fixed English-US primary layout. For secondary-layout
    /// characters (Cyrillic, etc.), use [`KeyboardMap::letter_event_from_char`] or
    /// [`LanguageBundle::letter_event_from_char`] which consult the actually loaded
    /// keyboard map.
    pub fn from_char(value: char) -> Option<Self> {
        let lower = value.to_lowercase().next().unwrap_or(value);
        match lower {
            'a' => Some(Self::A),
            'b' => Some(Self::B),
            'c' => Some(Self::C),
            'd' => Some(Self::D),
            'e' => Some(Self::E),
            'f' => Some(Self::F),
            'g' => Some(Self::G),
            'h' => Some(Self::H),
            'i' => Some(Self::I),
            'j' => Some(Self::J),
            'k' => Some(Self::K),
            'l' => Some(Self::L),
            'm' => Some(Self::M),
            'n' => Some(Self::N),
            'o' => Some(Self::O),
            'p' => Some(Self::P),
            'q' => Some(Self::Q),
            'r' => Some(Self::R),
            's' => Some(Self::S),
            't' => Some(Self::T),
            'u' => Some(Self::U),
            'v' => Some(Self::V),
            'w' => Some(Self::W),
            'x' => Some(Self::X),
            'y' => Some(Self::Y),
            'z' => Some(Self::Z),
            '`' => Some(Self::Grave),
            '[' => Some(Self::LBracket),
            ']' => Some(Self::RBracket),
            ';' => Some(Self::Semicolon),
            '\'' => Some(Self::Quote),
            ',' => Some(Self::Comma),
            '.' => Some(Self::Period),
            '~' => Some(Self::Grave),
            '{' => Some(Self::LBracket),
            '}' => Some(Self::RBracket),
            ':' => Some(Self::Semicolon),
            '"' => Some(Self::Quote),
            '<' => Some(Self::Comma),
            '>' => Some(Self::Period),

            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct KeyboardMap {
    unshifted: [char; PhysicalKey::COUNT],
    shifted: [char; PhysicalKey::COUNT],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyboardMapError {
    row: &'static str,
    expected: usize,
    actual: usize,
}

impl KeyboardMapError {
    fn new(row: &'static str, actual: usize) -> Self {
        Self {
            row,
            expected: PhysicalKey::COUNT,
            actual,
        }
    }
}

impl std::fmt::Display for KeyboardMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} keyboard row has {} characters; expected {}",
            self.row, self.actual, self.expected
        )
    }
}

impl std::error::Error for KeyboardMapError {}

impl KeyboardMap {
    pub fn new(unshifted: [char; PhysicalKey::COUNT], shifted: [char; PhysicalKey::COUNT]) -> Self {
        Self { unshifted, shifted }
    }

    pub fn from_rows(unshifted: &str, shifted: &str) -> Result<Self, KeyboardMapError> {
        Ok(Self::new(
            parse_keyboard_row("unshifted", unshifted)?,
            parse_keyboard_row("shifted", shifted)?,
        ))
    }

    pub fn named(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "english-us" | "en-us" | "us" => Some(Self::english_us()),
            "russian-jcuken" | "ru-jcuken" | "jcuken" => Some(Self::russian_jcuken()),
            _ => None,
        }
    }

    pub fn english_us() -> Self {
        Self::new(ENGLISH_US_UNSHIFTED, ENGLISH_US_SHIFTED)
    }

    pub fn russian_jcuken() -> Self {
        Self::new(RUSSIAN_JCUKEN_UNSHIFTED, RUSSIAN_JCUKEN_SHIFTED)
    }

    pub fn render(&self, key: PhysicalKey, shift: bool) -> char {
        let index = key.index();
        if shift {
            self.shifted[index]
        } else {
            self.unshifted[index]
        }
    }

    pub fn letter_event_from_char(&self, value: char) -> Option<LetterEvent> {
        let lower = value.to_lowercase().next().unwrap_or(value);
        for index in 0..PhysicalKey::COUNT {
            let key = PhysicalKey::from_index(index as u8).expect("keyboard map index is valid");
            if self.unshifted[index] == lower {
                return Some(LetterEvent {
                    physical_key: key,
                    shift: value.is_uppercase(),
                });
            }
            if self.shifted[index] == value {
                return Some(LetterEvent {
                    physical_key: key,
                    shift: true,
                });
            }
        }

        None
    }
}

fn parse_keyboard_row(
    row: &'static str,
    value: &str,
) -> Result<[char; PhysicalKey::COUNT], KeyboardMapError> {
    let chars: Vec<char> = value.chars().collect();
    let actual = chars.len();
    chars
        .try_into()
        .map_err(|_| KeyboardMapError::new(row, actual))
}

const ENGLISH_US_UNSHIFTED: [char; PhysicalKey::COUNT] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', '`', '[', ']', ';', '\'', ',', '.',
];

const ENGLISH_US_SHIFTED: [char; PhysicalKey::COUNT] = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S',
    'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '~', '{', '}', ':', '"', '<', '>',
];

const RUSSIAN_JCUKEN_UNSHIFTED: [char; PhysicalKey::COUNT] = [
    'ф', 'и', 'с', 'в', 'у', 'а', 'п', 'р', 'ш', 'о', 'л', 'д', 'ь', 'т', 'щ', 'з', 'й', 'к', 'ы',
    'е', 'г', 'м', 'ц', 'ч', 'н', 'я', 'ё', 'х', 'ъ', 'ж', 'э', 'б', 'ю',
];

const RUSSIAN_JCUKEN_SHIFTED: [char; PhysicalKey::COUNT] = [
    'Ф', 'И', 'С', 'В', 'У', 'А', 'П', 'Р', 'Ш', 'О', 'Л', 'Д', 'Ь', 'Т', 'Щ', 'З', 'Й', 'К', 'Ы',
    'Е', 'Г', 'М', 'Ц', 'Ч', 'Н', 'Я', 'Ё', 'Х', 'Ъ', 'Ж', 'Э', 'Б', 'Ю',
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LetterEvent {
    pub physical_key: PhysicalKey,
    pub shift: bool,
}

impl LetterEvent {
    pub fn new(physical_key: PhysicalKey) -> Self {
        Self {
            physical_key,
            shift: false,
        }
    }

    /// Converts an English-US layout character to a `LetterEvent` with shift detection.
    ///
    /// Returns `None` for characters outside the English-US layout (including
    /// all Cyrillic). For secondary-layout characters, use
    /// [`LanguageBundle::letter_event_from_char`] instead.
    pub fn from_char(value: char) -> Option<Self> {
        let physical_key = PhysicalKey::from_char(value)?;
        let shift = match value {
            '~' | '{' | '}' | ':' | '"' | '<' | '>' => true,
            _ => value.is_uppercase(),
        };

        Some(Self {
            physical_key,
            shift,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEvent {
    Letter(LetterEvent),
    Literal(char),
    Backspace,
    EndToken,
    HostBypass,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LayoutCandidates {
    pub english: String,
    pub secondary: String,
}

impl LayoutCandidates {
    pub fn get(&self, layout: Layout) -> &str {
        match layout {
            Layout::English => &self.english,
            Layout::Secondary => &self.secondary,
        }
    }
}

pub fn render_letters_with_bundle(
    letters: &[LetterEvent],
    layout: Layout,
    bundle: &LanguageBundle,
) -> String {
    let pack = bundle.pack(layout);
    letters
        .iter()
        .map(|event| pack.keyboard.render(event.physical_key, event.shift))
        .collect()
}

pub fn render_candidates_with_bundle(
    letters: &[LetterEvent],
    bundle: &LanguageBundle,
) -> LayoutCandidates {
    LayoutCandidates {
        english: render_letters_with_bundle(letters, Layout::English, bundle),
        secondary: render_letters_with_bundle(letters, Layout::Secondary, bundle),
    }
}

impl LanguageBundle {
    pub fn render(&self, event: LetterEvent, layout: Layout) -> char {
        self.pack(layout)
            .keyboard
            .render(event.physical_key, event.shift)
    }

    pub fn letter_event_from_char(&self, value: char) -> Option<LetterEvent> {
        self.english
            .keyboard
            .letter_event_from_char(value)
            .or_else(|| self.secondary.keyboard.letter_event_from_char(value))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Keep,
    Bypass,
    Use(Layout),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Keep,
    Commit(char),
    ReplaceToken {
        old_len: usize,
        replacement: String,
        layout: Layout,
    },
    ResetToken,
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutScore {
    pub layout: Layout,
    pub total: f32,
    pub bigram: f32,
    pub trigram: f32,
    pub dict_exact_bonus: f32,
    pub dict_prefix_bonus: f32,
    pub exact_count: u64,
    pub prefix_sum: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct ScoreAnalysis {
    pub english: LayoutScore,
    pub secondary: LayoutScore,
}

impl ScoreAnalysis {
    pub fn margin_for(&self, layout: Layout) -> f32 {
        match layout {
            Layout::English => self.english.total - self.secondary.total,
            Layout::Secondary => self.secondary.total - self.english.total,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EngineOutput {
    pub candidates: LayoutCandidates,
    pub score: ScoreAnalysis,
    pub decision: Decision,
    pub action: Action,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HostContext {
    pub secure_input: bool,
    pub app_excluded: bool,
}

impl HostContext {
    pub fn bypasses_engine(self) -> bool {
        self.secure_input || self.app_excluded
    }
}

/// Runtime configuration for the engine.
#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    /// Tokens shorter than this never trigger a layout switch.
    pub min_token_len: usize,
    /// Required score margin (log10 probability units) before switching.
    pub confidence_margin: f32,
    /// Bonus weight added when the rendered token is a complete dictionary entry.
    pub dict_exact_weight: f32,
    /// Bonus weight added when the rendered token is a prefix of dictionary entries.
    pub dict_prefix_weight: f32,
    /// Higher margin required when the winning candidate has no dictionary evidence.
    pub ngram_only_confidence_margin: f32,
    /// Multiplier applied to the bigram log-probability sum.
    pub bigram_weight: f32,
    /// Multiplier applied to the trigram log-probability sum.
    pub trigram_weight: f32,
    /// Length-normalize the n-gram score (divide by token char count).
    pub length_normalize: bool,
    /// Refuse to switch on tokens with internal capitals (camelCase).
    pub disable_on_internal_caps: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            min_token_len: 4,
            // With length_normalize = true the score is per-character log-prob, so
            // 1.0 means "winning language is on average 10x more likely per bigram".
            confidence_margin: 1.0,
            dict_exact_weight: 5.0,
            dict_prefix_weight: 2.0,
            ngram_only_confidence_margin: 3.0,
            bigram_weight: 1.0,
            trigram_weight: 1.0,
            length_normalize: true,
            disable_on_internal_caps: true,
        }
    }
}

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

fn score_layout(
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

/// True for characters that are NOT mapped to any physical key position in
/// either keyboard layout — digits and ASCII symbols that don't have a
/// Cyrillic letter on the same key.
///
/// Keyboard-position chars like `,` `.` `;` `'` `[` `]` `` ` `` and their
/// shifted forms `<` `>` `:` `"` `{` `}` `~` are intentionally NOT in this
/// list: in the Russian layout the same physical keys produce Cyrillic
/// letters (б, ю, ж, э, х, ъ, ё, …). The engine must see them as Letter
/// events so it can score `юбка` against `.,rf` and pick the right side.
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
                | '\\'
                | '?'
                | '|'
        )
}

#[cfg(test)]
mod tests {
    use super::{
        Action, Decision, Engine, EngineConfig, HostContext, InputEvent, KeyboardMap, Layout,
        LetterEvent, PhysicalKey,
    };
    use crate::data::LanguageBundle;

    fn fixture_bundle() -> LanguageBundle {
        LanguageBundle::for_testing(
            &[
                ("hello", 1000),
                ("typeflow", 50),
                ("type", 800),
                ("flow", 600),
                ("the", 5000),
                ("and", 4000),
                ("input", 700),
                ("language", 300),
                ("http", 2000),
                ("https", 1800),
                ("json", 1700),
                ("aws", 1600),
                ("kubectl", 1500),
                ("terraform", 1400),
                ("token", 1300),
                ("secret", 1200),
                ("password", 1100),
                ("bearer", 1000),
                ("namespace", 900),
                ("deployment", 800),
            ],
            &[
                ("привет", 900),
                ("привычка", 200),
                ("приватный", 150),
                ("мир", 1000),
                ("язык", 600),
                ("раскладка", 100),
                ("клавиатура", 80),
                ("переключение", 70),
            ],
        )
    }

    fn engine() -> Engine {
        Engine::new(EngineConfig::default(), fixture_bundle())
    }

    fn engine_with_config(config: EngineConfig) -> Engine {
        Engine::new(config, fixture_bundle())
    }

    #[test]
    fn it_defaults_to_english() {
        let engine = engine();
        assert_eq!(engine.current_layout(), Layout::English);
        assert_eq!(engine.token_len(), 0);
        assert_eq!(engine.bundle().display_name(Layout::English), "English");
        assert_eq!(engine.bundle().display_name(Layout::Secondary), "Russian");
    }

    #[test]
    fn it_tracks_token_candidates_from_letter_events() {
        let mut engine = engine();
        for event in letters(&[
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::B,
            PhysicalKey::D,
            PhysicalKey::T,
            PhysicalKey::N,
        ]) {
            engine.process(InputEvent::Letter(event));
        }

        let candidates = engine.token_candidates();
        assert_eq!(candidates.english, "ghbdtn");
        assert_eq!(candidates.secondary, "привет");
    }

    #[test]
    fn it_scores_russian_higher_for_privet() {
        let mut engine = engine();
        for event in letters(&[
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::B,
            PhysicalKey::D,
            PhysicalKey::T,
            PhysicalKey::N,
        ]) {
            engine.process(InputEvent::Letter(event));
        }
        let score = engine.score(&engine.token_candidates());
        assert!(
            score.secondary.total > score.english.total,
            "expected russian > english, got {:?} vs {:?}",
            score.secondary,
            score.english
        );
    }

    #[test]
    fn it_scores_english_higher_for_typeflow() {
        let mut engine = engine();
        for event in letters(&[
            PhysicalKey::T,
            PhysicalKey::Y,
            PhysicalKey::P,
            PhysicalKey::E,
            PhysicalKey::F,
            PhysicalKey::L,
            PhysicalKey::O,
            PhysicalKey::W,
        ]) {
            engine.process(InputEvent::Letter(event));
        }
        let score = engine.score(&engine.token_candidates());
        assert!(score.english.total > score.secondary.total);
    }

    #[test]
    fn it_replaces_token_when_decision_switches_layout() {
        let mut engine = engine();
        let mut last_action = Action::Keep;
        for event in letters(&[
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::B,
            PhysicalKey::D,
            PhysicalKey::T,
            PhysicalKey::N,
        ]) {
            last_action = engine.process(InputEvent::Letter(event)).action;
        }

        // Engine should have flipped to Russian at some point during the token.
        assert_eq!(engine.current_layout(), Layout::Secondary);
        // The final action should either be a Commit (already in Russian) or Replace
        // (just flipped on this letter); both are acceptable depending on calibration.
        assert!(matches!(
            last_action,
            Action::Commit(_) | Action::ReplaceToken { .. }
        ));
    }

    #[test]
    fn it_keeps_layout_for_short_tokens() {
        let mut engine = engine();
        engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::G)));
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::H)));
        assert_eq!(output.decision, Decision::Keep);
        assert_eq!(output.action, Action::Commit('h'));
    }

    #[test]
    fn it_resets_token_on_end_token() {
        let mut engine = engine();
        engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::T)));
        let output = engine.process(InputEvent::EndToken);
        assert_eq!(output.action, Action::ResetToken);
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn it_pops_token_on_backspace() {
        let mut engine = engine();
        for event in letters(&[PhysicalKey::T, PhysicalKey::Y, PhysicalKey::P]) {
            engine.process(InputEvent::Letter(event));
        }
        let output = engine.process(InputEvent::Backspace);
        assert_eq!(output.action, Action::Keep);
        assert_eq!(engine.token_len(), 2);
        assert_eq!(engine.token_candidates().english, "ty");
    }

    #[test]
    fn it_reverts_layout_when_backspacing_before_the_switch_point() {
        let mut engine = engine();
        for event in letters(&[
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::B,
            PhysicalKey::D,
        ]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.current_layout(), Layout::Secondary);

        engine.process(InputEvent::Backspace);

        assert_eq!(engine.token_candidates().english, "ghb");
        assert_eq!(engine.current_layout(), Layout::English);
    }

    #[test]
    fn backspace_on_empty_token_is_a_noop() {
        let mut engine = engine();
        let output = engine.process(InputEvent::Backspace);
        assert_eq!(output.action, Action::Keep);
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn it_refuses_to_switch_on_internal_caps() {
        let mut engine = engine();
        // Type "gHbDtN" — same physical keys as привет but with mid-word capitals.
        // Engine should refuse to switch layouts because this looks like an identifier.
        for (idx, key) in [
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::B,
            PhysicalKey::D,
            PhysicalKey::T,
            PhysicalKey::N,
        ]
        .iter()
        .copied()
        .enumerate()
        {
            engine.process(InputEvent::Letter(LetterEvent {
                physical_key: key,
                // shift on every odd-indexed letter (positions 1, 3, 5)
                shift: idx % 2 == 1,
            }));
        }
        assert_eq!(engine.current_layout(), Layout::English);
    }

    #[test]
    fn it_does_not_block_capitalized_first_letter() {
        let mut engine = engine();
        // Type "Привет" via physical keys with shift on position 0 only.
        for (idx, key) in [
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::B,
            PhysicalKey::D,
            PhysicalKey::T,
            PhysicalKey::N,
        ]
        .iter()
        .copied()
        .enumerate()
        {
            engine.process(InputEvent::Letter(LetterEvent {
                physical_key: key,
                shift: idx == 0,
            }));
        }
        assert_eq!(engine.current_layout(), Layout::Secondary);

        let score = engine.score(&engine.token_candidates());
        assert!(score.secondary.exact_count > 0);
    }

    #[test]
    fn it_renders_shifted_english_punctuation_positions() {
        let bundle = fixture_bundle();
        let cases = [
            ('~', PhysicalKey::Grave),
            ('{', PhysicalKey::LBracket),
            ('}', PhysicalKey::RBracket),
            (':', PhysicalKey::Semicolon),
            ('"', PhysicalKey::Quote),
            ('<', PhysicalKey::Comma),
            ('>', PhysicalKey::Period),
        ];

        for (character, key) in cases {
            let event = LetterEvent::from_char(character).unwrap();

            assert_eq!(event.physical_key, key);
            assert!(event.shift);
            assert_eq!(bundle.render(event, Layout::English), character);
        }
    }

    #[test]
    fn it_requires_stronger_margin_without_dictionary_evidence() {
        let config = EngineConfig {
            confidence_margin: 0.0,
            ngram_only_confidence_margin: f32::MAX,
            ..EngineConfig::default()
        };

        let mut engine = engine_with_config(config);
        for event in letters(&[
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::B,
            PhysicalKey::D,
            PhysicalKey::T,
            PhysicalKey::N,
        ]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.current_layout(), Layout::Secondary);
        engine.process(InputEvent::EndToken);

        for event in letters(&[
            PhysicalKey::Q,
            PhysicalKey::W,
            PhysicalKey::E,
            PhysicalKey::R,
            PhysicalKey::T,
            PhysicalKey::Y,
        ]) {
            engine.process(InputEvent::Letter(event));
        }

        assert_eq!(engine.current_layout(), Layout::Secondary);
    }

    #[test]
    fn literal_resets_token_and_commits_the_character() {
        let mut engine = engine();
        for event in letters(&[PhysicalKey::G, PhysicalKey::H, PhysicalKey::B]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.token_len(), 3);

        let output = engine.process(InputEvent::Literal('1'));
        assert_eq!(output.action, Action::Commit('1'));
        assert_eq!(engine.token_len(), 0);

        // The remaining letters form a new short token; engine stays English
        // because each segment is below min_token_len.
        for event in letters(&[PhysicalKey::D, PhysicalKey::T, PhysicalKey::N]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.current_layout(), Layout::English);
    }

    #[test]
    fn backspace_after_literal_keeps_token_state_consistent() {
        // Reproduces the desync that existed when literals didn't terminate the
        // token: the host buffer would drift from `engine.token` because each
        // literal added a character to the host while leaving `engine.token`
        // unchanged. With literal = reset+commit, both sides advance in lockstep.
        let mut engine = engine();
        let mut committed = String::new();

        for character in "abc".chars() {
            let action = engine.process_action(input_event_for_char(&engine, character));
            apply_action_to_string(&action, &mut committed);
        }
        let action = engine.process_action(InputEvent::Literal('1'));
        apply_action_to_string(&action, &mut committed);
        for character in "def".chars() {
            let action = engine.process_action(input_event_for_char(&engine, character));
            apply_action_to_string(&action, &mut committed);
        }

        assert_eq!(committed, "abc1def");
        assert_eq!(engine.token_len(), 3); // engine sees only "def"

        // Backspace 3 times via the engine; the host pops the corresponding
        // char itself (Backspace returns Action::Keep).
        for _ in 0..3 {
            engine.process_action(InputEvent::Backspace);
            committed.pop();
        }
        assert_eq!(engine.token_len(), 0);
        assert_eq!(committed, "abc1");
    }

    #[test]
    fn it_bypasses_acronym_like_tokens() {
        let mut engine = engine();
        let mut output = engine.process(InputEvent::Letter(LetterEvent {
            physical_key: PhysicalKey::H,
            shift: true,
        }));
        for key in [PhysicalKey::T, PhysicalKey::T, PhysicalKey::P] {
            output = engine.process(InputEvent::Letter(LetterEvent {
                physical_key: key,
                shift: true,
            }));
        }

        assert_eq!(output.decision, Decision::Bypass);
        assert_eq!(engine.current_layout(), Layout::English);
    }

    #[test]
    fn it_keeps_devops_and_secret_like_tokens_english() {
        let cases = [
            "http",
            "https://example.com",
            "user@example.com",
            "/var/log/nginx/access.log",
            "snake_case",
            "camelCase",
            "HTTP",
            "abc123",
            "CLOUDACCESSKEYIDLIKEVALUE1234",
            "arn:aws:iam::123456789012:role/Admin",
            "kubectl",
            "terraform",
            "Bearer",
            "ghp_abcd1234TOKEN",
            "password123!",
            "kube-system",
            "deployment.apps",
        ];

        for token in cases {
            let mut engine = engine();
            let rendered = run_cli_like_token(&mut engine, token);

            assert_eq!(
                engine.current_layout(),
                Layout::English,
                "false positive for {token}, rendered {rendered}"
            );
            assert_eq!(rendered, token);
        }
    }

    #[test]
    fn weird_unicode_literals_do_not_panic_or_switch() {
        let mut engine = engine();
        let mut committed = String::new();

        for character in ['🧪', '\u{200d}', '\u{0301}', '\n', '\u{0000}', 'ß'] {
            let output = engine.process(InputEvent::Literal(character));
            apply_action_to_string(&output.action, &mut committed);
        }

        assert_eq!(engine.current_layout(), Layout::English);
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn host_context_bypasses_secure_and_excluded_inputs() {
        let mut engine = engine();
        for event in letters(&[PhysicalKey::G, PhysicalKey::H, PhysicalKey::B]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.token_len(), 3);

        engine.set_host_context(HostContext {
            secure_input: true,
            app_excluded: false,
        });
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::D)));

        assert_eq!(output.decision, Decision::Bypass);
        assert_eq!(output.action, Action::Keep);
        assert_eq!(engine.token_len(), 0);

        engine.set_host_context(HostContext {
            secure_input: false,
            app_excluded: true,
        });
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::D)));
        assert_eq!(output.action, Action::Keep);
    }

    #[test]
    fn modifier_bypass_event_does_not_commit_or_score() {
        let mut engine = engine();
        let output = engine.process(InputEvent::HostBypass);

        assert_eq!(output.decision, Decision::Bypass);
        assert_eq!(output.action, Action::Keep);
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn action_only_path_matches_full_output_for_switching_token() {
        let mut full = engine();
        let mut fast = engine();
        let mut full_committed = String::new();
        let mut fast_committed = String::new();

        for character in "ghbdtn".chars() {
            let input = input_event_for_char(&full, character);
            let output = full.process(input);
            apply_action_to_string(&output.action, &mut full_committed);

            let input = input_event_for_char(&fast, character);
            let action = fast.process_action(input);
            apply_action_to_string(&action, &mut fast_committed);
        }

        assert_eq!(full_committed, "привет");
        assert_eq!(fast_committed, full_committed);
        assert_eq!(fast.current_layout(), full.current_layout());
    }

    #[test]
    fn language_bundle_reverse_maps_secondary_characters() {
        let engine = engine();
        let event = engine.letter_event_from_char('ж').unwrap();

        assert_eq!(event.physical_key, PhysicalKey::Semicolon);
        assert_eq!(engine.bundle().render(event, Layout::Secondary), 'ж');
    }

    #[test]
    fn keyboard_map_loads_named_and_custom_layouts() {
        let named = KeyboardMap::named("russian-jcuken").unwrap();
        assert_eq!(named.render(PhysicalKey::G, false), 'п');

        let custom = KeyboardMap::from_rows(
            "abcdefghijklmnopqrstuvwxyz`[];',.",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ~{}:\"<>",
        )
        .unwrap();
        assert_eq!(custom.render(PhysicalKey::A, false), 'a');
        assert_eq!(custom.render(PhysicalKey::Period, true), '>');
    }

    #[test]
    fn replace_token_old_len_matches_committed_prefix() {
        // When the engine flips mid-stream, the host has only committed the
        // previous letters of the token (one Commit per letter). `old_len` must
        // equal that committed-prefix length so AppKit's
        // `client.insertText(_:replacementRange:)` can target a real range
        // instead of going off the end of the buffer.
        let mut engine = engine();
        let mut committed = String::new();
        let mut flip_action: Option<Action> = None;

        for character in "ghbdtn".chars() {
            let action = engine.process_action(input_event_for_char(&engine, character));
            if matches!(action, Action::ReplaceToken { .. }) {
                flip_action = Some(action.clone());
                let committed_before_flip = committed.chars().count();
                if let Action::ReplaceToken { old_len, .. } = &action {
                    assert_eq!(
                        *old_len, committed_before_flip,
                        "old_len ({old_len}) must equal the host's committed prefix \
                         ({committed_before_flip}) at the moment of the flip"
                    );
                }
            }
            apply_action_to_string(&action, &mut committed);
        }

        assert_eq!(committed, "привет");
        assert!(
            flip_action.is_some(),
            "expected a ReplaceToken event for ghbdtn -> привет"
        );
    }

    #[test]
    fn force_switch_old_len_matches_committed_token() {
        // For force_switch, every letter of the token has already been
        // committed; old_len must equal the full token length (no off-by-one).
        let mut engine = engine();
        let mut committed = String::new();
        for character in "type".chars() {
            let action = engine.process_action(input_event_for_char(&engine, character));
            apply_action_to_string(&action, &mut committed);
        }
        assert_eq!(committed.chars().count(), 4);

        let output = engine.force_switch_token();
        let Action::ReplaceToken { old_len, .. } = output.action else {
            panic!("expected ReplaceToken from force_switch");
        };
        assert_eq!(old_len, 4);
    }

    #[test]
    fn it_force_switches_the_current_token() {
        let mut engine = engine();
        for event in letters(&[
            PhysicalKey::T,
            PhysicalKey::Y,
            PhysicalKey::P,
            PhysicalKey::E,
        ]) {
            engine.process(InputEvent::Letter(event));
        }

        let output = engine.force_switch_token();

        assert_eq!(engine.current_layout(), Layout::Secondary);
        assert_eq!(
            output.action,
            Action::ReplaceToken {
                old_len: 4,
                replacement: "ензу".to_owned(),
                layout: Layout::Secondary,
            }
        );
    }

    fn letters(physical_keys: &[PhysicalKey]) -> Vec<LetterEvent> {
        physical_keys
            .iter()
            .copied()
            .map(LetterEvent::new)
            .collect()
    }

    fn run_cli_like_token(engine: &mut Engine, token: &str) -> String {
        let mut committed = String::new();
        for character in token.chars() {
            let input = input_event_for_char(engine, character);
            let action = engine.process_action(input);
            apply_action_to_string(&action, &mut committed);
        }
        committed
    }

    fn input_event_for_char(engine: &Engine, character: char) -> InputEvent {
        engine.input_event_from_char(character)
    }

    fn apply_action_to_string(action: &Action, committed: &mut String) {
        match action {
            Action::Keep | Action::ResetToken => {}
            Action::Commit(character) => committed.push(*character),
            Action::ReplaceToken {
                old_len,
                replacement,
                ..
            } => {
                for _ in 0..*old_len {
                    committed.pop();
                }
                committed.push_str(replacement);
            }
        }
    }
}
