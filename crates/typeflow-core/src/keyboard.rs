use crate::data::LanguageBundle;
use crate::{Layout, LayoutCandidates};

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
    Backslash,
}

impl PhysicalKey {
    pub const COUNT: usize = 34;

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
            33 => Some(Self::Backslash),
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
            '\\' => Some(Self::Backslash),
            ';' => Some(Self::Semicolon),
            '\'' => Some(Self::Quote),
            ',' => Some(Self::Comma),
            '.' => Some(Self::Period),
            '~' => Some(Self::Grave),
            '{' => Some(Self::LBracket),
            '}' => Some(Self::RBracket),
            '|' => Some(Self::Backslash),
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
            "ukrainian-jcuken-osx" | "uk-jcuken-osx" | "ukrainian-osx" | "uk-osx" => {
                Some(Self::ukrainian_jcuken_osx())
            }
            _ => None,
        }
    }

    pub fn english_us() -> Self {
        Self::new(ENGLISH_US_UNSHIFTED, ENGLISH_US_SHIFTED)
    }

    pub fn ukrainian_jcuken_osx() -> Self {
        Self::new(UKRAINIAN_JCUKEN_OSX_UNSHIFTED, UKRAINIAN_JCUKEN_OSX_SHIFTED)
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
            let Some(key) = PhysicalKey::from_index(index as u8) else {
                continue;
            };
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
    't', 'u', 'v', 'w', 'x', 'y', 'z', '`', '[', ']', ';', '\'', ',', '.', '\\',
];

const ENGLISH_US_SHIFTED: [char; PhysicalKey::COUNT] = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S',
    'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '~', '{', '}', ':', '"', '<', '>', '|',
];

const UKRAINIAN_JCUKEN_OSX_UNSHIFTED: [char; PhysicalKey::COUNT] = [
    'ф', 'і', 'с', 'в', 'у', 'а', 'п', 'р', 'ш', 'о', 'л', 'д', 'ь', 'т', 'щ', 'з', 'й', 'к', 'и',
    'е', 'г', 'м', 'ц', 'ч', 'н', 'я', '\'', 'х', 'ї', 'ж', 'є', 'б', 'ю', 'ґ',
];

const UKRAINIAN_JCUKEN_OSX_SHIFTED: [char; PhysicalKey::COUNT] = [
    'Ф', 'І', 'С', 'В', 'У', 'А', 'П', 'Р', 'Ш', 'О', 'Л', 'Д', 'Ь', 'Т', 'Щ', 'З', 'Й', 'К', 'И',
    'Е', 'Г', 'М', 'Ц', 'Ч', 'Н', 'Я', '~', 'Х', 'Ї', 'Ж', 'Є', 'Б', 'Ю', 'Ґ',
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
            '~' | '{' | '}' | ':' | '"' | '<' | '>' | '|' => true,
            _ => value.is_uppercase(),
        };

        Some(Self {
            physical_key,
            shift,
        })
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
