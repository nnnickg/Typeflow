use crate::LetterEvent;

/// Maximum configured token length accepted by supported hosts.
///
/// FFI replacement payloads use a 4096-byte buffer. Since a Unicode scalar can
/// occupy up to four UTF-8 bytes, 1024 tracked letters is the largest value
/// that cannot overflow that payload solely from token length.
pub const MAX_CONFIG_TOKEN_LEN: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layout {
    English,
    /// The configurable non-English side of the active language pair.
    Secondary,
}

impl Layout {
    pub const SECONDARY: Self = Self::Secondary;
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
    /// Maximum number of letter events tracked as one replaceable token.
    pub max_token_len: usize,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineConfigError {
    MinTokenLenZero,
    MaxTokenLenZero,
    MinTokenLenGreaterThanMaxTokenLen { min: usize, max: usize },
    MaxTokenLenTooLarge { value: usize, max: usize },
    InvalidFloat { field: &'static str },
}

impl std::fmt::Display for EngineConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MinTokenLenZero => write!(f, "min_token_len must be greater than zero"),
            Self::MaxTokenLenZero => write!(f, "max_token_len must be greater than zero"),
            Self::MinTokenLenGreaterThanMaxTokenLen { min, max } => write!(
                f,
                "min_token_len ({min}) must be less than or equal to max_token_len ({max})"
            ),
            Self::MaxTokenLenTooLarge { value, max } => write!(
                f,
                "max_token_len ({value}) must be less than or equal to {max}"
            ),
            Self::InvalidFloat { field } => {
                write!(
                    f,
                    "{field} must be finite and greater than or equal to zero"
                )
            }
        }
    }
}

impl std::error::Error for EngineConfigError {}

impl EngineConfig {
    pub fn validate(self) -> Result<(), EngineConfigError> {
        if self.min_token_len == 0 {
            return Err(EngineConfigError::MinTokenLenZero);
        }
        if self.max_token_len == 0 {
            return Err(EngineConfigError::MaxTokenLenZero);
        }
        if self.min_token_len > self.max_token_len {
            return Err(EngineConfigError::MinTokenLenGreaterThanMaxTokenLen {
                min: self.min_token_len,
                max: self.max_token_len,
            });
        }
        if self.max_token_len > MAX_CONFIG_TOKEN_LEN {
            return Err(EngineConfigError::MaxTokenLenTooLarge {
                value: self.max_token_len,
                max: MAX_CONFIG_TOKEN_LEN,
            });
        }

        for (field, value) in [
            ("confidence_margin", self.confidence_margin),
            ("dict_exact_weight", self.dict_exact_weight),
            ("dict_prefix_weight", self.dict_prefix_weight),
            (
                "ngram_only_confidence_margin",
                self.ngram_only_confidence_margin,
            ),
            ("bigram_weight", self.bigram_weight),
            ("trigram_weight", self.trigram_weight),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(EngineConfigError::InvalidFloat { field });
            }
        }

        Ok(())
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            min_token_len: 4,
            max_token_len: 128,
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
