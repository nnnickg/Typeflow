use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use fst::automaton::{Automaton, Str};
use fst::{IntoStreamer, Streamer};
use serde::{Deserialize, Serialize};

use crate::{KeyboardMap, KeyboardMapError, Layout};

pub const PACK_FORMAT_VERSION: u32 = 1;
pub const PACK_MANIFEST_FILE: &str = "pack.toml";
pub const PACK_NGRAMS_FILE: &str = "ngrams.bin";
pub const PACK_DICT_FILE: &str = "dict.fst";

/// Compact on-disk representation produced by the `typeflow-data` xtask.
///
/// Serialized via `bincode` and shipped under `crates/typeflow-core/data/`.
/// Sorted vectors keep the format diff-friendly when artifacts are checked into the repo.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledLanguageData {
    pub language_tag: String,
    pub bigrams: Vec<(String, f32)>,
    pub trigrams: Vec<(String, f32)>,
    pub bigram_floor: f32,
    pub trigram_floor: f32,
}

/// Runtime form of `CompiledLanguageData`. Built once at engine creation.
pub struct LanguageModel {
    pub language_tag: String,
    pub bigram: HashMap<String, f32>,
    pub trigram: HashMap<String, f32>,
    pub bigram_floor: f32,
    pub trigram_floor: f32,
}

impl LanguageModel {
    pub fn from_compiled(compiled: CompiledLanguageData) -> Self {
        Self {
            language_tag: compiled.language_tag,
            bigram: compiled.bigrams.into_iter().collect(),
            trigram: compiled.trigrams.into_iter().collect(),
            bigram_floor: compiled.bigram_floor,
            trigram_floor: compiled.trigram_floor,
        }
    }

    pub fn from_bincode(bytes: &[u8]) -> Result<Self, bincode::Error> {
        let compiled: CompiledLanguageData = bincode::deserialize(bytes)?;
        Ok(Self::from_compiled(compiled))
    }

    pub fn bigram_log_prob(&self, bigram: &str) -> f32 {
        self.bigram
            .get(bigram)
            .copied()
            .unwrap_or(self.bigram_floor)
    }

    pub fn trigram_log_prob(&self, trigram: &str) -> f32 {
        self.trigram
            .get(trigram)
            .copied()
            .unwrap_or(self.trigram_floor)
    }

    pub fn score_bigrams(&self, text: &str) -> f32 {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() < 2 {
            return 0.0;
        }
        chars
            .windows(2)
            .map(|window| {
                let key: String = window.iter().collect();
                self.bigram_log_prob(&key)
            })
            .sum()
    }

    pub fn score_trigrams(&self, text: &str) -> f32 {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() < 3 {
            return 0.0;
        }
        chars
            .windows(3)
            .map(|window| {
                let key: String = window.iter().collect();
                self.trigram_log_prob(&key)
            })
            .sum()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KeyboardManifest {
    pub unshifted: String,
    pub shifted: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LanguagePackManifest {
    pub format_version: u32,
    pub id: String,
    pub display_name: String,
    pub script: String,
    pub layout: String,
    pub ngrams: PathBuf,
    pub dict: PathBuf,
    pub source_corpus: Option<String>,
    pub source_dictionary: Option<String>,
    pub build_id: Option<String>,
    pub keyboard: Option<KeyboardManifest>,
}

impl LanguagePackManifest {
    pub fn embedded_ukrainian() -> Self {
        Self {
            format_version: PACK_FORMAT_VERSION,
            id: "uk".to_owned(),
            display_name: "Ukrainian".to_owned(),
            script: "Cyrillic".to_owned(),
            layout: "ukrainian-jcuken-osx".to_owned(),
            ngrams: PathBuf::from(PACK_NGRAMS_FILE),
            dict: PathBuf::from(PACK_DICT_FILE),
            source_corpus: Some("OPUS OpenSubtitles2018 mono".to_owned()),
            source_dictionary: Some("hermitdave/FrequencyWords 2018".to_owned()),
            build_id: Some("embedded".to_owned()),
            keyboard: None,
        }
    }

    pub fn read_from_dir(pack_dir: &Path) -> Result<Self, BundleError> {
        let manifest_path = pack_dir.join(PACK_MANIFEST_FILE);
        let manifest_text = fs::read_to_string(&manifest_path)?;
        let manifest: Self = toml::from_str(&manifest_text)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn write_to_dir(&self, pack_dir: &Path) -> Result<(), BundleError> {
        self.validate()?;
        fs::create_dir_all(pack_dir)?;
        let manifest_text = toml::to_string_pretty(self)?;
        fs::write(pack_dir.join(PACK_MANIFEST_FILE), manifest_text)?;
        Ok(())
    }

    pub fn artifact_paths(&self, pack_dir: &Path) -> Result<(PathBuf, PathBuf), BundleError> {
        Ok((
            resolve_pack_file(pack_dir, &self.ngrams)?,
            resolve_pack_file(pack_dir, &self.dict)?,
        ))
    }

    pub fn normalized_for_install(&self) -> Self {
        let mut manifest = self.clone();
        manifest.ngrams = PathBuf::from(PACK_NGRAMS_FILE);
        manifest.dict = PathBuf::from(PACK_DICT_FILE);
        manifest
    }

    pub fn keyboard_map(&self) -> Result<KeyboardMap, BundleError> {
        if let Some(keyboard) = &self.keyboard {
            return Ok(KeyboardMap::from_rows(
                keyboard.unshifted.as_str(),
                keyboard.shifted.as_str(),
            )?);
        }

        KeyboardMap::named(&self.layout).ok_or_else(|| {
            BundleError::InvalidPack(format!(
                "unknown keyboard layout '{}'; use a named built-in layout or provide [keyboard] rows",
                self.layout
            ))
        })
    }

    fn validate(&self) -> Result<(), BundleError> {
        if self.format_version != PACK_FORMAT_VERSION {
            return Err(BundleError::InvalidPack(format!(
                "unsupported pack format {}; expected {}",
                self.format_version, PACK_FORMAT_VERSION
            )));
        }
        validate_pack_id(self.id.as_str())?;
        require_non_empty("display_name", self.display_name.as_str())?;
        require_non_empty("script", self.script.as_str())?;
        require_non_empty("layout", self.layout.as_str())?;
        validate_pack_relative_path("ngrams", &self.ngrams)?;
        validate_pack_relative_path("dict", &self.dict)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct PackMetadata {
    pub format_version: u32,
    pub source_corpus: String,
    pub source_dictionary: String,
    pub build_id: String,
    pub ngram_bytes: usize,
    pub dict_bytes: usize,
    pub fingerprint: u64,
}

impl PackMetadata {
    fn from_bytes(ngram_bytes: &[u8], dict_bytes: &[u8]) -> Self {
        let mut fingerprint = fnv1a64(ngram_bytes);
        fingerprint = fnv1a64_extend(fingerprint, dict_bytes);

        Self {
            format_version: 1,
            source_corpus: "OPUS OpenSubtitles2018 mono".to_owned(),
            source_dictionary: "hermitdave/FrequencyWords 2018".to_owned(),
            build_id: "embedded".to_owned(),
            ngram_bytes: ngram_bytes.len(),
            dict_bytes: dict_bytes.len(),
            fingerprint,
        }
    }
}

pub struct LanguagePack {
    pub id: String,
    pub display_name: String,
    pub script: String,
    pub keyboard_layout: String,
    pub keyboard: KeyboardMap,
    pub model: LanguageModel,
    pub dict: fst::Map<Vec<u8>>,
    pub metadata: PackMetadata,
}

impl LanguagePack {
    pub fn from_bytes(
        id: &str,
        display_name: &str,
        script: &str,
        keyboard_layout: &str,
        keyboard: KeyboardMap,
        ngrams: &[u8],
        dict_bytes: Vec<u8>,
    ) -> Result<Self, BundleError> {
        let compiled: CompiledLanguageData = bincode::deserialize(ngrams)?;
        if compiled.language_tag != id {
            return Err(BundleError::LanguageTagMismatch {
                expected: id.to_owned(),
                actual: compiled.language_tag,
            });
        }
        let metadata = PackMetadata::from_bytes(ngrams, dict_bytes.as_slice());
        Ok(Self {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            script: script.to_owned(),
            keyboard_layout: keyboard_layout.to_owned(),
            keyboard,
            model: LanguageModel::from_compiled(compiled),
            dict: fst::Map::new(dict_bytes)?,
            metadata,
        })
    }

    pub fn from_pack_dir(pack_dir: &Path) -> Result<Self, BundleError> {
        let manifest = LanguagePackManifest::read_from_dir(pack_dir)?;
        let keyboard = manifest.keyboard_map()?;
        let (ngrams_path, dict_path) = manifest.artifact_paths(pack_dir)?;
        let ngrams = fs::read(&ngrams_path)?;
        let dict_bytes = fs::read(&dict_path)?;

        let mut pack = Self::from_bytes(
            manifest.id.as_str(),
            manifest.display_name.as_str(),
            manifest.script.as_str(),
            manifest.layout.as_str(),
            keyboard,
            ngrams.as_slice(),
            dict_bytes,
        )?;

        pack.metadata.format_version = manifest.format_version;
        pack.metadata.source_corpus = manifest
            .source_corpus
            .unwrap_or_else(|| "external pack".to_owned());
        pack.metadata.source_dictionary = manifest
            .source_dictionary
            .unwrap_or_else(|| "external pack".to_owned());
        pack.metadata.build_id = manifest.build_id.unwrap_or_else(|| "external".to_owned());

        Ok(pack)
    }
}

/// Bundle of all static language data needed to drive one English<->secondary pair.
pub struct LanguageBundle {
    pub english: LanguagePack,
    pub secondary: LanguagePack,
}

#[derive(Debug)]
pub enum BundleError {
    Io(io::Error),
    Bincode(bincode::Error),
    Fst(fst::Error),
    TomlDe(toml::de::Error),
    TomlSer(toml::ser::Error),
    KeyboardMap(KeyboardMapError),
    LanguageTagMismatch { expected: String, actual: String },
    InvalidPack(String),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::Io(error) => write!(f, "io error: {error}"),
            BundleError::Bincode(error) => write!(f, "bincode error: {error}"),
            BundleError::Fst(error) => write!(f, "fst error: {error}"),
            BundleError::TomlDe(error) => write!(f, "toml parse error: {error}"),
            BundleError::TomlSer(error) => write!(f, "toml serialize error: {error}"),
            BundleError::KeyboardMap(error) => write!(f, "keyboard map error: {error}"),
            BundleError::LanguageTagMismatch { expected, actual } => write!(
                f,
                "language tag mismatch: manifest id is '{expected}', ngram artifact is '{actual}'"
            ),
            BundleError::InvalidPack(error) => write!(f, "invalid pack: {error}"),
        }
    }
}

impl std::error::Error for BundleError {}

impl From<io::Error> for BundleError {
    fn from(value: io::Error) -> Self {
        BundleError::Io(value)
    }
}

impl From<bincode::Error> for BundleError {
    fn from(value: bincode::Error) -> Self {
        BundleError::Bincode(value)
    }
}

impl From<fst::Error> for BundleError {
    fn from(value: fst::Error) -> Self {
        BundleError::Fst(value)
    }
}

impl From<toml::de::Error> for BundleError {
    fn from(value: toml::de::Error) -> Self {
        BundleError::TomlDe(value)
    }
}

impl From<toml::ser::Error> for BundleError {
    fn from(value: toml::ser::Error) -> Self {
        BundleError::TomlSer(value)
    }
}

impl From<KeyboardMapError> for BundleError {
    fn from(value: KeyboardMapError) -> Self {
        BundleError::KeyboardMap(value)
    }
}

impl LanguageBundle {
    /// Loads the language bundle embedded into the binary at compile time.
    ///
    /// The raw subtitle/frequency downloads are build-time inputs only. Runtime code should
    /// normally use this path so the CLI/IMK bundle is self-contained.
    pub fn embedded() -> Result<Self, BundleError> {
        let (en_ngrams, en_dict) = Self::embedded_english_artifacts();
        let (secondary_ngrams, secondary_dict) = Self::embedded_secondary_artifacts();
        Self::from_bytes(
            en_ngrams,
            secondary_ngrams,
            en_dict.to_vec(),
            secondary_dict.to_vec(),
        )
    }

    pub fn embedded_english_artifacts() -> (&'static [u8], &'static [u8]) {
        (
            include_bytes!("../data/en.ngrams.bin"),
            include_bytes!("../data/en.dict.fst"),
        )
    }

    pub fn embedded_secondary_artifacts() -> (&'static [u8], &'static [u8]) {
        (
            include_bytes!("../data/uk.ngrams.bin"),
            include_bytes!("../data/uk.dict.fst"),
        )
    }

    /// Loads a bundle from the four artifacts produced by `typeflow-data` in `data_dir`:
    /// `en.ngrams.bin`, `uk.ngrams.bin`, `en.dict.fst`, `uk.dict.fst`.
    pub fn from_data_dir(data_dir: &Path) -> Result<Self, BundleError> {
        let en_ngrams = fs::read(data_dir.join("en.ngrams.bin"))?;
        let secondary_ngrams = fs::read(data_dir.join("uk.ngrams.bin"))?;
        let en_dict = fs::read(data_dir.join("en.dict.fst"))?;
        let secondary_dict = fs::read(data_dir.join("uk.dict.fst"))?;

        Self::from_bytes(&en_ngrams, &secondary_ngrams, en_dict, secondary_dict)
    }

    /// Loads a bundle from in-memory bytes. Suitable for `include_bytes!` use from
    /// downstream crates (CLI / FFI / IMK bundle).
    pub fn from_bytes(
        en_ngrams: &[u8],
        secondary_ngrams: &[u8],
        en_dict: Vec<u8>,
        secondary_dict: Vec<u8>,
    ) -> Result<Self, BundleError> {
        Ok(Self {
            english: LanguagePack::from_bytes(
                "en",
                "English",
                "Latin",
                "english-us",
                KeyboardMap::english_us(),
                en_ngrams,
                en_dict,
            )?,
            secondary: LanguagePack::from_bytes(
                "uk",
                "Ukrainian",
                "Cyrillic",
                "ukrainian-jcuken-osx",
                KeyboardMap::ukrainian_jcuken_osx(),
                secondary_ngrams,
                secondary_dict,
            )?,
        })
    }

    pub fn with_secondary_pack(secondary: LanguagePack) -> Result<Self, BundleError> {
        if secondary.id == "en" {
            return Err(BundleError::InvalidPack(
                "secondary pack id cannot be 'en'; English is the fixed primary side".to_owned(),
            ));
        }

        let (en_ngrams, en_dict) = Self::embedded_english_artifacts();
        Ok(Self {
            english: LanguagePack::from_bytes(
                "en",
                "English",
                "Latin",
                "english-us",
                KeyboardMap::english_us(),
                en_ngrams,
                en_dict.to_vec(),
            )?,
            secondary,
        })
    }

    pub fn from_secondary_pack_dir(pack_dir: &Path) -> Result<Self, BundleError> {
        Self::with_secondary_pack(LanguagePack::from_pack_dir(pack_dir)?)
    }

    pub fn pack(&self, layout: Layout) -> &LanguagePack {
        match layout {
            Layout::English => &self.english,
            Layout::Secondary => &self.secondary,
        }
    }

    pub fn display_name(&self, layout: Layout) -> &str {
        self.pack(layout).display_name.as_str()
    }
}

impl LanguageBundle {
    /// Builds a bundle from inline word lists for use in unit tests. N-gram tables
    /// are derived directly from the supplied word/frequency pairs, so tests are
    /// fully deterministic and do not depend on any on-disk artifacts.
    pub fn for_testing(english_words: &[(&str, u64)], secondary_words: &[(&str, u64)]) -> Self {
        Self {
            english: synthetic_pack(
                "en",
                "English",
                "Latin",
                "english-us",
                KeyboardMap::english_us(),
                english_words,
            ),
            secondary: synthetic_pack(
                "uk",
                "Ukrainian",
                "Cyrillic",
                "ukrainian-jcuken-osx",
                KeyboardMap::ukrainian_jcuken_osx(),
                secondary_words,
            ),
        }
    }
}

fn synthetic_pack(
    id: &str,
    display_name: &str,
    script: &str,
    keyboard_layout: &str,
    keyboard: KeyboardMap,
    words: &[(&str, u64)],
) -> LanguagePack {
    LanguagePack {
        id: id.to_owned(),
        display_name: display_name.to_owned(),
        script: script.to_owned(),
        keyboard_layout: keyboard_layout.to_owned(),
        keyboard,
        model: synthetic_model(id, words),
        dict: synthetic_fst(words),
        metadata: PackMetadata {
            format_version: 1,
            source_corpus: "test".to_owned(),
            source_dictionary: "test".to_owned(),
            build_id: "test".to_owned(),
            ngram_bytes: 0,
            dict_bytes: 0,
            fingerprint: 0,
        },
    }
}

fn synthetic_model(tag: &str, words: &[(&str, u64)]) -> LanguageModel {
    let mut bigrams: HashMap<String, u64> = HashMap::new();
    let mut trigrams: HashMap<String, u64> = HashMap::new();

    for (word, count) in words {
        let chars: Vec<char> = word.chars().collect();
        for window in chars.windows(2) {
            let key: String = window.iter().collect();
            *bigrams.entry(key).or_insert(0) += *count;
        }
        for window in chars.windows(3) {
            let key: String = window.iter().collect();
            *trigrams.entry(key).or_insert(0) += *count;
        }
    }

    let bigram_total: u64 = bigrams.values().sum();
    let trigram_total: u64 = trigrams.values().sum();
    let bigram_v = bigrams.len().max(1) as f32;
    let trigram_v = trigrams.len().max(1) as f32;

    let bigram_floor = (1.0_f32 / (bigram_total as f32 + bigram_v)).log10();
    let trigram_floor = (1.0_f32 / (trigram_total as f32 + trigram_v)).log10();

    let bigram = bigrams
        .into_iter()
        .map(|(key, count)| {
            let p = (count as f32 + 1.0) / (bigram_total as f32 + bigram_v);
            (key, p.log10())
        })
        .collect();

    let trigram = trigrams
        .into_iter()
        .map(|(key, count)| {
            let p = (count as f32 + 1.0) / (trigram_total as f32 + trigram_v);
            (key, p.log10())
        })
        .collect();

    LanguageModel {
        language_tag: tag.to_owned(),
        bigram,
        trigram,
        bigram_floor,
        trigram_floor,
    }
}

fn synthetic_fst(words: &[(&str, u64)]) -> fst::Map<Vec<u8>> {
    let mut sorted: Vec<(&str, u64)> = words.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut builder = fst::MapBuilder::memory();
    for (word, count) in sorted {
        builder.insert(word.as_bytes(), count).unwrap();
    }
    let bytes = builder.into_inner().unwrap();
    fst::Map::new(bytes).unwrap()
}

fn require_non_empty(field: &str, value: &str) -> Result<(), BundleError> {
    if value.trim().is_empty() {
        return Err(BundleError::InvalidPack(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

fn validate_pack_id(id: &str) -> Result<(), BundleError> {
    require_non_empty("id", id)?;
    if !id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(BundleError::InvalidPack(format!(
            "id '{id}' contains unsupported characters; use ASCII letters, digits, '-' or '_'"
        )));
    }
    Ok(())
}

fn validate_pack_relative_path(field: &str, path: &Path) -> Result<(), BundleError> {
    if path.as_os_str().is_empty() {
        return Err(BundleError::InvalidPack(format!("{field} path is empty")));
    }
    if path.is_absolute() {
        return Err(BundleError::InvalidPack(format!(
            "{field} path must be relative to the pack directory"
        )));
    }

    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BundleError::InvalidPack(format!(
                    "{field} path must stay inside the pack directory"
                )));
            }
        }
    }

    Ok(())
}

fn resolve_pack_file(pack_dir: &Path, relative: &Path) -> Result<PathBuf, BundleError> {
    validate_pack_relative_path("artifact", relative)?;
    Ok(pack_dir.join(relative))
}

/// Outcome of a dictionary lookup against a single language's FST.
#[derive(Clone, Copy, Debug, Default)]
pub struct DictLookup {
    /// Frequency of an exact match, if the token is itself a dictionary entry.
    pub exact_count: u64,
    /// Sum of frequencies for the first `prefix_sample` entries that start with the token.
    pub prefix_sum: u64,
    /// Number of dictionary entries scanned during the prefix walk (capped).
    pub prefix_sample: usize,
}

const PREFIX_SAMPLE_CAP: usize = 256;

pub fn dict_lookup(token: &str, dict: &fst::Map<Vec<u8>>) -> DictLookup {
    let mut result = DictLookup::default();

    if let Some(count) = dict.get(token.as_bytes()) {
        result.exact_count = count;
    }

    let auto = Str::new(token).starts_with();
    let mut stream = dict.search(&auto).into_stream();
    let mut count = 0;
    while let Some((_, value)) = stream.next() {
        result.prefix_sum = result.prefix_sum.saturating_add(value);
        count += 1;
        if count >= PREFIX_SAMPLE_CAP {
            break;
        }
    }
    result.prefix_sample = count;
    result
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_extend(0xcbf29ce484222325, bytes)
}

fn fnv1a64_extend(mut state: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x100000001b3);
    }
    state
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        CompiledLanguageData, LanguagePack, LanguagePackManifest, PACK_DICT_FILE,
        PACK_FORMAT_VERSION, PACK_MANIFEST_FILE, PACK_NGRAMS_FILE,
    };

    #[test]
    fn pack_manifest_rejects_path_traversal() {
        let dir = temp_pack_dir("path-traversal");
        fs::write(
            dir.join(PACK_MANIFEST_FILE),
            r#"
format_version = 1
id = "xx"
display_name = "Bad"
script = "Latin"
layout = "english-us"
ngrams = "../ngrams.bin"
dict = "dict.fst"
"#,
        )
        .unwrap();

        let error = LanguagePackManifest::read_from_dir(&dir).unwrap_err();
        assert!(error.to_string().contains("stay inside the pack directory"));
    }

    #[test]
    fn pack_loader_rejects_malformed_ngram_bytes() {
        let dir = temp_pack_dir("bad-ngrams");
        write_manifest(&dir, "xx");
        fs::write(dir.join(PACK_NGRAMS_FILE), b"not bincode").unwrap();
        fs::write(dir.join(PACK_DICT_FILE), valid_fst_bytes()).unwrap();

        let error = LanguagePack::from_pack_dir(&dir).err().unwrap();
        assert!(error.to_string().contains("bincode"));
    }

    #[test]
    fn pack_loader_rejects_language_tag_mismatch() {
        let dir = temp_pack_dir("tag-mismatch");
        write_manifest(&dir, "xx");
        fs::write(dir.join(PACK_NGRAMS_FILE), valid_ngram_bytes("yy")).unwrap();
        fs::write(dir.join(PACK_DICT_FILE), valid_fst_bytes()).unwrap();

        let error = LanguagePack::from_pack_dir(&dir).err().unwrap();
        assert!(error.to_string().contains("language tag mismatch"));
    }

    #[test]
    fn pack_loader_rejects_malformed_fst_bytes() {
        let dir = temp_pack_dir("bad-fst");
        write_manifest(&dir, "xx");
        fs::write(dir.join(PACK_NGRAMS_FILE), valid_ngram_bytes("xx")).unwrap();
        fs::write(dir.join(PACK_DICT_FILE), b"not an fst").unwrap();

        let error = LanguagePack::from_pack_dir(&dir).err().unwrap();
        assert!(error.to_string().contains("fst"));
    }

    fn write_manifest(dir: &std::path::Path, id: &str) {
        fs::write(
            dir.join(PACK_MANIFEST_FILE),
            format!(
                r#"
format_version = {PACK_FORMAT_VERSION}
id = "{id}"
display_name = "Test"
script = "Latin"
layout = "english-us"
ngrams = "{PACK_NGRAMS_FILE}"
dict = "{PACK_DICT_FILE}"
"#
            ),
        )
        .unwrap();
    }

    fn valid_ngram_bytes(language_tag: &str) -> Vec<u8> {
        bincode::serialize(&CompiledLanguageData {
            language_tag: language_tag.to_owned(),
            bigrams: vec![("ab".to_owned(), -0.1)],
            trigrams: vec![("abc".to_owned(), -0.1)],
            bigram_floor: -5.0,
            trigram_floor: -6.0,
        })
        .unwrap()
    }

    fn valid_fst_bytes() -> Vec<u8> {
        let mut builder = fst::MapBuilder::memory();
        builder.insert("abc", 1).unwrap();
        builder.into_inner().unwrap()
    }

    fn temp_pack_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "typeflow-core-{name}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
