use std::borrow::Cow;
#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::str;
use std::sync::{Arc, OnceLock};

use fst::{IntoStreamer, Streamer};
use serde::{Deserialize, Serialize};

use crate::{KeyboardMap, KeyboardMapError, Layout};

pub const PACK_FORMAT_VERSION: u32 = 4;
pub const PACK_MANIFEST_FILE: &str = "pack.toml";
pub const PACK_NGRAMS_FILE: &str = "ngrams.bin";
pub const PACK_DICT_FILE: &str = "dict.fst";
pub const PACK_DICT_PREFIX_FILE: &str = "dict-prefix.bin";
const NGRAM_MAGIC: &[u8; 8] = b"TFNG0002";
const DICT_PREFIX_MAGIC: &[u8; 8] = b"TFPX0001";
const EMBEDDED_UKRAINIAN_PUNCTUATION_LETTER_KEYS: &str = "`[]\\;',.~{}|:\"<>";
const MAX_NGRAM_STRING_BYTES: usize = 256;
const MAX_NGRAM_ENTRIES: usize = 10_000_000;

/// Compact on-disk representation produced by the `typeclaw-data` xtask.
///
/// Encoded as a TypeClaw n-gram artifact and shipped under `crates/typeclaw-core/data/`.
/// Sorted vectors keep the format diff-friendly when artifacts are checked into the repo.
#[derive(Clone, Debug)]
pub struct CompiledLanguageData {
    pub language_tag: String,
    pub bigrams: Vec<(String, f32)>,
    pub trigrams: Vec<(String, f32)>,
    pub bigram_floor: f32,
    pub trigram_floor: f32,
}

#[derive(Debug)]
pub enum NgramDecodeError {
    InvalidMagic,
    UnexpectedEof {
        needed: usize,
        remaining: usize,
    },
    InvalidUtf8(str::Utf8Error),
    LengthTooLarge {
        field: &'static str,
        len: usize,
        max: usize,
    },
    TrailingBytes {
        read: usize,
        total: usize,
    },
}

impl std::fmt::Display for NgramDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NgramDecodeError::InvalidMagic => write!(f, "invalid n-gram artifact magic"),
            NgramDecodeError::UnexpectedEof { needed, remaining } => write!(
                f,
                "unexpected end of n-gram artifact: needed {needed} bytes, had {remaining}"
            ),
            NgramDecodeError::InvalidUtf8(error) => {
                write!(f, "invalid UTF-8 in n-gram artifact: {error}")
            }
            NgramDecodeError::LengthTooLarge { field, len, max } => write!(
                f,
                "n-gram artifact field '{field}' is too large: {len} bytes/items, max {max}"
            ),
            NgramDecodeError::TrailingBytes { read, total } => write!(
                f,
                "trailing bytes after n-gram artifact: read {read} of {total}"
            ),
        }
    }
}

impl std::error::Error for NgramDecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NgramDecodeError::InvalidUtf8(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum NgramEncodeError {
    LengthTooLarge {
        field: &'static str,
        len: usize,
        max: usize,
    },
}

impl std::fmt::Display for NgramEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NgramEncodeError::LengthTooLarge { field, len, max } => write!(
                f,
                "n-gram artifact field '{field}' is too large: {len} bytes/items, max {max}"
            ),
        }
    }
}

impl std::error::Error for NgramEncodeError {}

#[derive(Debug)]
pub enum DictPrefixDecodeError {
    InvalidMagic,
    Fst(fst::Error),
    UnexpectedEof {
        needed: usize,
        remaining: usize,
    },
    InvalidUtf8(str::Utf8Error),
    LengthTooLarge {
        field: &'static str,
        len: usize,
        max: usize,
    },
    EmptyPrefix,
    PrefixesNotSorted,
    PrefixSampleTooLarge {
        sample: u64,
        max: usize,
    },
    TrailingBytes {
        read: usize,
        total: usize,
    },
}

impl std::fmt::Display for DictPrefixDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "invalid dictionary prefix artifact magic"),
            Self::Fst(error) => write!(f, "dictionary prefix FST error: {error}"),
            Self::UnexpectedEof { needed, remaining } => write!(
                f,
                "unexpected end of dictionary prefix artifact: needed {needed} bytes, had {remaining}"
            ),
            Self::InvalidUtf8(error) => {
                write!(f, "invalid UTF-8 in dictionary prefix artifact: {error}")
            }
            Self::LengthTooLarge { field, len, max } => write!(
                f,
                "dictionary prefix artifact field '{field}' is too large: {len} bytes/items, max {max}"
            ),
            Self::EmptyPrefix => write!(f, "dictionary prefix artifact contains an empty prefix"),
            Self::PrefixesNotSorted => write!(
                f,
                "dictionary prefix artifact prefixes must be strictly sorted"
            ),
            Self::PrefixSampleTooLarge { sample, max } => write!(
                f,
                "dictionary prefix artifact sample count {sample} exceeds cap {max}"
            ),
            Self::TrailingBytes { read, total } => write!(
                f,
                "trailing bytes after dictionary prefix artifact: read {read} of {total}"
            ),
        }
    }
}

impl std::error::Error for DictPrefixDecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Fst(error) => Some(error),
            Self::InvalidUtf8(error) => Some(error),
            _ => None,
        }
    }
}

impl From<fst::Error> for DictPrefixDecodeError {
    fn from(value: fst::Error) -> Self {
        Self::Fst(value)
    }
}

#[derive(Debug)]
pub enum DictPrefixEncodeError {
    Fst(fst::Error),
    LengthTooLarge {
        field: &'static str,
        len: usize,
        max: usize,
    },
}

impl std::fmt::Display for DictPrefixEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fst(error) => write!(f, "dictionary prefix FST error: {error}"),
            Self::LengthTooLarge { field, len, max } => write!(
                f,
                "dictionary prefix artifact field '{field}' is too large: {len} bytes/items, max {max}"
            ),
        }
    }
}

impl std::error::Error for DictPrefixEncodeError {}

impl From<fst::Error> for DictPrefixEncodeError {
    fn from(value: fst::Error) -> Self {
        Self::Fst(value)
    }
}

/// Runtime form of `CompiledLanguageData`. Built once at engine creation.
pub struct LanguageModel {
    pub language_tag: String,
    pub bigram: Vec<(u64, f32)>,
    pub trigram: Vec<(u64, f32)>,
    pub bigram_floor: f32,
    pub trigram_floor: f32,
}

/// Precomputed dictionary prefix evidence for one immutable word-frequency FST.
///
/// Building this once at pack load keeps keystroke scoring out of the dictionary
/// stream walker. Values match the capped lexicographic prefix sample used by
/// `dict_lookup`: for each prefix, sum only the first `PREFIX_SAMPLE_CAP`
/// dictionary entries that start with that prefix.
pub struct DictionaryIndex {
    storage: DictionaryIndexStorage,
}

enum DictionaryIndexStorage {
    Entries(Vec<PrefixIndexEntry>),
    Fst {
        prefix_sum: fst::Map<Cow<'static, [u8]>>,
        prefix_sample: fst::Map<Cow<'static, [u8]>>,
    },
}

#[derive(Clone)]
struct PrefixIndexEntry {
    prefix: Vec<u8>,
    sum: u64,
    sample: u64,
}

impl DictionaryIndex {
    pub fn from_dict<D: AsRef<[u8]>>(dict: &fst::Map<D>) -> Self {
        Self {
            storage: DictionaryIndexStorage::Entries(build_prefix_index_entries(dict)),
        }
    }

    pub fn from_artifact_bytes(bytes: &[u8]) -> Result<Self, DictPrefixDecodeError> {
        let (prefix_sum, prefix_sample) = dictionary_index_fst_slices(bytes)?;
        Ok(Self {
            storage: DictionaryIndexStorage::Fst {
                prefix_sum: fst::Map::new(Cow::Owned(prefix_sum.to_vec()))?,
                prefix_sample: fst::Map::new(Cow::Owned(prefix_sample.to_vec()))?,
            },
        })
    }

    pub fn from_static_artifact_bytes(bytes: &'static [u8]) -> Result<Self, DictPrefixDecodeError> {
        let (prefix_sum, prefix_sample) = dictionary_index_fst_slices(bytes)?;
        Ok(Self {
            storage: DictionaryIndexStorage::Fst {
                prefix_sum: fst::Map::new(Cow::Borrowed(prefix_sum))?,
                prefix_sample: fst::Map::new(Cow::Borrowed(prefix_sample))?,
            },
        })
    }

    pub fn to_artifact_bytes(&self) -> Result<Vec<u8>, DictPrefixEncodeError> {
        encode_dictionary_index(self)
    }

    pub fn len(&self) -> usize {
        match &self.storage {
            DictionaryIndexStorage::Entries(entries) => entries.len(),
            DictionaryIndexStorage::Fst { prefix_sum, .. } => prefix_sum.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn lookup<D: AsRef<[u8]>>(&self, token: &str, dict: &fst::Map<D>) -> DictLookup {
        let bytes = token.as_bytes();
        match &self.storage {
            DictionaryIndexStorage::Entries(entries) => {
                let prefix = entries
                    .binary_search_by(|entry| entry.prefix.as_slice().cmp(bytes))
                    .ok()
                    .map(|idx| &entries[idx]);

                DictLookup {
                    exact_count: dict.get(bytes).unwrap_or(0),
                    prefix_sum: prefix.map(|entry| entry.sum).unwrap_or(0),
                    prefix_sample: prefix
                        .map(|entry| entry.sample.min(usize::MAX as u64) as usize)
                        .unwrap_or(0),
                }
            }
            DictionaryIndexStorage::Fst {
                prefix_sum,
                prefix_sample,
            } => DictLookup {
                exact_count: dict.get(bytes).unwrap_or(0),
                prefix_sum: prefix_sum.get(bytes).unwrap_or(0),
                prefix_sample: prefix_sample
                    .get(bytes)
                    .map(|value| value.min(usize::MAX as u64) as usize)
                    .unwrap_or(0),
            },
        }
    }
}

pub fn encode_dictionary_index(index: &DictionaryIndex) -> Result<Vec<u8>, DictPrefixEncodeError> {
    let (prefix_sum, prefix_sample);
    let (prefix_sum_bytes, prefix_sample_bytes) = match &index.storage {
        DictionaryIndexStorage::Entries(entries) => {
            prefix_sum = dictionary_index_fst(entries, |entry| entry.sum)?;
            prefix_sample = dictionary_index_fst(entries, |entry| entry.sample)?;
            (
                prefix_sum.as_fst().as_bytes(),
                prefix_sample.as_fst().as_bytes(),
            )
        }
        DictionaryIndexStorage::Fst {
            prefix_sum,
            prefix_sample,
        } => (
            prefix_sum.as_fst().as_bytes(),
            prefix_sample.as_fst().as_bytes(),
        ),
    };

    let mut bytes = Vec::new();
    bytes.extend_from_slice(DICT_PREFIX_MAGIC);
    push_dict_prefix_blob(&mut bytes, prefix_sum_bytes);
    push_dict_prefix_blob(&mut bytes, prefix_sample_bytes);
    Ok(bytes)
}

fn dictionary_index_fst(
    entries: &[PrefixIndexEntry],
    value: impl Fn(&PrefixIndexEntry) -> u64,
) -> Result<fst::Map<Vec<u8>>, fst::Error> {
    fst::Map::from_iter(
        entries
            .iter()
            .map(|entry| (entry.prefix.as_slice(), value(entry))),
    )
}

fn dictionary_index_fst_slices(bytes: &[u8]) -> Result<(&[u8], &[u8]), DictPrefixDecodeError> {
    let mut reader = DictPrefixReader::new(bytes);
    if reader.read_exact(DICT_PREFIX_MAGIC.len())? != DICT_PREFIX_MAGIC {
        return Err(DictPrefixDecodeError::InvalidMagic);
    }
    let prefix_sum = reader.read_blob()?;
    let prefix_sample = reader.read_blob()?;
    reader.finish()?;
    Ok((prefix_sum, prefix_sample))
}

fn push_dict_prefix_blob(bytes: &mut Vec<u8>, blob: &[u8]) {
    bytes.extend_from_slice(&(blob.len() as u64).to_le_bytes());
    bytes.extend_from_slice(blob);
}

fn build_prefix_index_entries<D: AsRef<[u8]>>(dict: &fst::Map<D>) -> Vec<PrefixIndexEntry> {
    let mut active = Vec::<PrefixIndexEntry>::new();
    let mut entries = Vec::<PrefixIndexEntry>::new();
    let mut stream = dict.stream();

    while let Some((key, value)) = stream.next() {
        let Ok(text) = str::from_utf8(key) else {
            continue;
        };
        let bytes = text.as_bytes();
        let keep = active
            .iter()
            .take_while(|entry| bytes.starts_with(&entry.prefix))
            .count();
        entries.extend(active.drain(keep..));

        for (idx, end) in text
            .char_indices()
            .skip(1)
            .map(|(end, _)| end)
            .chain(std::iter::once(text.len()))
            .enumerate()
        {
            if idx >= keep {
                active.push(PrefixIndexEntry {
                    prefix: bytes[..end].to_vec(),
                    sum: 0,
                    sample: 0,
                });
            }
        }

        for entry in &mut active {
            record_prefix_sample(entry, value);
        }
    }

    entries.extend(active);
    entries.sort_unstable_by(|left, right| left.prefix.cmp(&right.prefix));
    entries
}

fn record_prefix_sample(entry: &mut PrefixIndexEntry, value: u64) {
    if entry.sample >= PREFIX_SAMPLE_CAP as u64 {
        return;
    }
    entry.sum = entry.sum.saturating_add(value);
    entry.sample += 1;
}

struct DictPrefixReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> DictPrefixReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_blob(&mut self) -> Result<&'a [u8], DictPrefixDecodeError> {
        let len = self.read_u64()? as usize;
        self.read_exact(len)
    }

    fn read_u64(&mut self) -> Result<u64, DictPrefixDecodeError> {
        let bytes = self.read_exact(8)?;
        let mut raw = [0u8; 8];
        raw.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(raw))
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], DictPrefixDecodeError> {
        let remaining = self.bytes.len().saturating_sub(self.position);
        if remaining < len {
            return Err(DictPrefixDecodeError::UnexpectedEof {
                needed: len,
                remaining,
            });
        }
        let start = self.position;
        self.position += len;
        Ok(&self.bytes[start..self.position])
    }

    fn finish(self) -> Result<(), DictPrefixDecodeError> {
        if self.position != self.bytes.len() {
            return Err(DictPrefixDecodeError::TrailingBytes {
                read: self.position,
                total: self.bytes.len(),
            });
        }
        Ok(())
    }
}

impl LanguageModel {
    pub fn from_compiled(compiled: CompiledLanguageData) -> Self {
        let mut bigram = compiled
            .bigrams
            .into_iter()
            .filter_map(|(key, score)| bigram_key(&key).map(|key| (key, score)))
            .collect::<Vec<_>>();
        let mut trigram = compiled
            .trigrams
            .into_iter()
            .filter_map(|(key, score)| trigram_key(&key).map(|key| (key, score)))
            .collect::<Vec<_>>();
        bigram.sort_unstable_by_key(|(key, _)| *key);
        trigram.sort_unstable_by_key(|(key, _)| *key);

        Self {
            language_tag: compiled.language_tag,
            bigram,
            trigram,
            bigram_floor: compiled.bigram_floor,
            trigram_floor: compiled.trigram_floor,
        }
    }

    pub fn from_artifact_bytes(bytes: &[u8]) -> Result<Self, NgramDecodeError> {
        let compiled = decode_compiled_language_data(bytes)?;
        Ok(Self::from_compiled(compiled))
    }

    pub fn bigram_log_prob(&self, bigram: u64) -> f32 {
        self.bigram
            .binary_search_by_key(&bigram, |(key, _)| *key)
            .map(|idx| self.bigram[idx].1)
            .unwrap_or(self.bigram_floor)
    }

    pub fn trigram_log_prob(&self, trigram: u64) -> f32 {
        self.trigram
            .binary_search_by_key(&trigram, |(key, _)| *key)
            .map(|idx| self.trigram[idx].1)
            .unwrap_or(self.trigram_floor)
    }

    pub fn bigram_log_prob_for_chars(&self, first: char, second: char) -> f32 {
        self.bigram_log_prob(encode_bigram(first, second))
    }

    pub fn trigram_log_prob_for_chars(&self, first: char, second: char, third: char) -> f32 {
        self.trigram_log_prob(encode_trigram(first, second, third))
    }

    pub fn score_ngrams(&self, text: &str) -> (f32, f32, usize) {
        let mut bigram = 0.0;
        let mut trigram = 0.0;
        let mut count = 0usize;
        let mut previous_two = '\0';
        let mut previous_one = '\0';

        for current in text.chars() {
            count += 1;
            if count >= 2 {
                bigram += self.bigram_log_prob(encode_bigram(previous_one, current));
            }
            if count >= 3 {
                trigram +=
                    self.trigram_log_prob(encode_trigram(previous_two, previous_one, current));
            }
            previous_two = previous_one;
            previous_one = current;
        }

        (bigram, trigram, count)
    }

    pub fn score_bigrams(&self, text: &str) -> f32 {
        self.score_ngrams(text).0
    }

    pub fn score_trigrams(&self, text: &str) -> f32 {
        self.score_ngrams(text).1
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
    #[serde(default)]
    pub punctuation_letter_keys: String,
    pub ngrams: PathBuf,
    pub dict: PathBuf,
    pub dict_prefix: PathBuf,
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
            punctuation_letter_keys: EMBEDDED_UKRAINIAN_PUNCTUATION_LETTER_KEYS.to_owned(),
            ngrams: PathBuf::from(PACK_NGRAMS_FILE),
            dict: PathBuf::from(PACK_DICT_FILE),
            dict_prefix: PathBuf::from(PACK_DICT_PREFIX_FILE),
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

    pub fn artifact_paths(
        &self,
        pack_dir: &Path,
    ) -> Result<(PathBuf, PathBuf, PathBuf), BundleError> {
        Ok((
            resolve_pack_file(pack_dir, &self.ngrams)?,
            resolve_pack_file(pack_dir, &self.dict)?,
            resolve_pack_file(pack_dir, &self.dict_prefix)?,
        ))
    }

    pub fn normalized_for_install(&self) -> Self {
        let mut manifest = self.clone();
        manifest.ngrams = PathBuf::from(PACK_NGRAMS_FILE);
        manifest.dict = PathBuf::from(PACK_DICT_FILE);
        manifest.dict_prefix = PathBuf::from(PACK_DICT_PREFIX_FILE);
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

    fn punctuation_letter_keys(&self, keyboard: &KeyboardMap) -> String {
        if !self.punctuation_letter_keys.is_empty() {
            return self.punctuation_letter_keys.clone();
        }
        keyboard.punctuation_letter_keys_against(&KeyboardMap::english_us())
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
        validate_punctuation_letter_keys(self.punctuation_letter_keys.as_str())?;
        validate_pack_relative_path("ngrams", &self.ngrams)?;
        validate_pack_relative_path("dict", &self.dict)?;
        validate_pack_relative_path("dict_prefix", &self.dict_prefix)?;
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
    pub dict_prefix_bytes: usize,
    pub fingerprint: u64,
}

impl PackMetadata {
    fn from_bytes(ngram_bytes: &[u8], dict_bytes: &[u8], dict_prefix_bytes: &[u8]) -> Self {
        let mut fingerprint = fnv1a64(ngram_bytes);
        fingerprint = fnv1a64_extend(fingerprint, dict_bytes);
        fingerprint = fnv1a64_extend(fingerprint, dict_prefix_bytes);

        Self {
            format_version: PACK_FORMAT_VERSION,
            source_corpus: "OPUS OpenSubtitles2018 mono".to_owned(),
            source_dictionary: "hermitdave/FrequencyWords 2018".to_owned(),
            build_id: "embedded".to_owned(),
            ngram_bytes: ngram_bytes.len(),
            dict_bytes: dict_bytes.len(),
            dict_prefix_bytes: dict_prefix_bytes.len(),
            fingerprint,
        }
    }
}

pub struct LanguagePack {
    pub id: String,
    pub display_name: String,
    pub script: String,
    pub keyboard_layout: String,
    pub punctuation_letter_keys: String,
    pub keyboard: KeyboardMap,
    pub model: LanguageModel,
    pub dict: fst::Map<Cow<'static, [u8]>>,
    pub dict_index: DictionaryIndex,
    pub metadata: PackMetadata,
}

struct LanguagePackDescriptor<'a> {
    id: &'a str,
    display_name: &'a str,
    script: &'a str,
    keyboard_layout: &'a str,
    punctuation_letter_keys: &'a str,
    keyboard: KeyboardMap,
}

pub struct DictionaryArtifacts<'a, D> {
    pub dict: D,
    pub prefix: DictionaryPrefixBytes<'a>,
}

pub enum DictionaryPrefixBytes<'a> {
    Borrowed(&'a [u8]),
    Static(&'static [u8]),
}

impl<'a> DictionaryPrefixBytes<'a> {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) | Self::Static(bytes) => bytes,
        }
    }

    fn decode_index(&self) -> Result<DictionaryIndex, DictPrefixDecodeError> {
        match self {
            Self::Borrowed(bytes) => DictionaryIndex::from_artifact_bytes(bytes),
            Self::Static(bytes) => DictionaryIndex::from_static_artifact_bytes(bytes),
        }
    }
}

impl LanguagePack {
    pub fn from_bytes<D>(
        id: &str,
        display_name: &str,
        script: &str,
        keyboard_layout: &str,
        keyboard: KeyboardMap,
        ngrams: &[u8],
        dict_bytes: D,
    ) -> Result<Self, BundleError>
    where
        D: Into<Cow<'static, [u8]>>,
    {
        Self::from_bytes_with_punctuation(
            LanguagePackDescriptor {
                id,
                display_name,
                script,
                keyboard_layout,
                punctuation_letter_keys: "",
                keyboard,
            },
            ngrams,
            dict_bytes,
        )
    }

    fn from_bytes_with_punctuation<D>(
        descriptor: LanguagePackDescriptor<'_>,
        ngrams: &[u8],
        dict_bytes: D,
    ) -> Result<Self, BundleError>
    where
        D: Into<Cow<'static, [u8]>>,
    {
        let dict_bytes = dict_bytes.into();
        let dict = fst::Map::new(dict_bytes)?;
        let dict_index = DictionaryIndex::from_dict(&dict);
        let dict_prefix_bytes = dict_index.to_artifact_bytes()?;
        Self::from_bytes_with_prefix_and_punctuation(
            descriptor,
            ngrams,
            DictionaryArtifacts {
                dict: Cow::Owned(dict.as_fst().as_bytes().to_vec()),
                prefix: DictionaryPrefixBytes::Borrowed(&dict_prefix_bytes),
            },
        )
    }

    fn from_bytes_with_prefix_and_punctuation<D>(
        descriptor: LanguagePackDescriptor<'_>,
        ngrams: &[u8],
        dictionary: DictionaryArtifacts<'_, D>,
    ) -> Result<Self, BundleError>
    where
        D: Into<Cow<'static, [u8]>>,
    {
        validate_punctuation_letter_keys(descriptor.punctuation_letter_keys)?;
        let dict_bytes = dictionary.dict.into();
        let compiled = decode_compiled_language_data(ngrams)?;
        if compiled.language_tag != descriptor.id {
            return Err(BundleError::LanguageTagMismatch {
                expected: descriptor.id.to_owned(),
                actual: compiled.language_tag,
            });
        }
        let dict = fst::Map::new(dict_bytes)?;
        let dict_index = dictionary.prefix.decode_index()?;
        let metadata = PackMetadata::from_bytes(
            ngrams,
            dict.as_fst().as_bytes(),
            dictionary.prefix.as_bytes(),
        );
        Ok(Self {
            id: descriptor.id.to_owned(),
            display_name: descriptor.display_name.to_owned(),
            script: descriptor.script.to_owned(),
            keyboard_layout: descriptor.keyboard_layout.to_owned(),
            punctuation_letter_keys: descriptor.punctuation_letter_keys.to_owned(),
            keyboard: descriptor.keyboard,
            model: LanguageModel::from_compiled(compiled),
            dict,
            dict_index,
            metadata,
        })
    }

    pub fn from_bytes_with_prefix<D>(
        id: &str,
        display_name: &str,
        script: &str,
        keyboard_layout: &str,
        keyboard: KeyboardMap,
        ngrams: &[u8],
        dictionary: DictionaryArtifacts<'_, D>,
    ) -> Result<Self, BundleError>
    where
        D: Into<Cow<'static, [u8]>>,
    {
        Self::from_bytes_with_prefix_and_punctuation(
            LanguagePackDescriptor {
                id,
                display_name,
                script,
                keyboard_layout,
                punctuation_letter_keys: "",
                keyboard,
            },
            ngrams,
            dictionary,
        )
    }

    pub fn from_pack_dir(pack_dir: &Path) -> Result<Self, BundleError> {
        reject_symlink(pack_dir, "pack directory")?;
        let manifest = LanguagePackManifest::read_from_dir(pack_dir)?;
        let keyboard = manifest.keyboard_map()?;
        let (ngrams_path, dict_path, dict_prefix_path) = manifest.artifact_paths(pack_dir)?;
        reject_symlink(&ngrams_path, "ngram artifact")?;
        reject_symlink(&dict_path, "dictionary artifact")?;
        reject_symlink(&dict_prefix_path, "dictionary prefix artifact")?;
        let ngrams = fs::read(&ngrams_path)?;
        let dict_bytes = fs::read(&dict_path)?;
        let dict_prefix_bytes = fs::read(&dict_prefix_path)?;

        let punctuation_letter_keys = manifest.punctuation_letter_keys(&keyboard);

        let mut pack = Self::from_bytes_with_prefix_and_punctuation(
            LanguagePackDescriptor {
                id: manifest.id.as_str(),
                display_name: manifest.display_name.as_str(),
                script: manifest.script.as_str(),
                keyboard_layout: manifest.layout.as_str(),
                punctuation_letter_keys: punctuation_letter_keys.as_str(),
                keyboard,
            },
            ngrams.as_slice(),
            DictionaryArtifacts {
                dict: Cow::Owned(dict_bytes),
                prefix: DictionaryPrefixBytes::Borrowed(dict_prefix_bytes.as_slice()),
            },
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
    Ngram(NgramDecodeError),
    DictPrefix(DictPrefixDecodeError),
    DictPrefixEncode(DictPrefixEncodeError),
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
            BundleError::Ngram(error) => write!(f, "ngram artifact error: {error}"),
            BundleError::DictPrefix(error) => {
                write!(f, "dictionary prefix artifact error: {error}")
            }
            BundleError::DictPrefixEncode(error) => {
                write!(f, "dictionary prefix artifact encode error: {error}")
            }
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

impl From<NgramDecodeError> for BundleError {
    fn from(value: NgramDecodeError) -> Self {
        BundleError::Ngram(value)
    }
}

impl From<DictPrefixDecodeError> for BundleError {
    fn from(value: DictPrefixDecodeError) -> Self {
        BundleError::DictPrefix(value)
    }
}

impl From<DictPrefixEncodeError> for BundleError {
    fn from(value: DictPrefixEncodeError) -> Self {
        BundleError::DictPrefixEncode(value)
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
    pub fn embedded_shared() -> Result<Arc<Self>, BundleError> {
        static EMBEDDED_LANGUAGE_BUNDLE: OnceLock<Arc<LanguageBundle>> = OnceLock::new();

        if let Some(bundle) = EMBEDDED_LANGUAGE_BUNDLE.get() {
            return Ok(Arc::clone(bundle));
        }

        let bundle = Arc::new(Self::embedded()?);
        if EMBEDDED_LANGUAGE_BUNDLE.set(Arc::clone(&bundle)).is_ok() {
            Ok(bundle)
        } else if let Some(existing) = EMBEDDED_LANGUAGE_BUNDLE.get() {
            Ok(Arc::clone(existing))
        } else {
            Ok(bundle)
        }
    }

    /// Loads the language bundle embedded into the binary at compile time.
    ///
    /// The raw subtitle/frequency downloads are build-time inputs only. Runtime code should
    /// normally use this path so the CLI/macOS agent is self-contained.
    pub fn embedded() -> Result<Self, BundleError> {
        let (en_ngrams, en_dict, en_dict_prefix) = Self::embedded_english_artifacts();
        let (secondary_ngrams, secondary_dict, secondary_dict_prefix) =
            Self::embedded_secondary_artifacts();
        Ok(Self {
            english: LanguagePack::from_bytes_with_prefix(
                "en",
                "English",
                "Latin",
                "english-us",
                KeyboardMap::english_us(),
                en_ngrams,
                DictionaryArtifacts {
                    dict: Cow::Borrowed(en_dict),
                    prefix: DictionaryPrefixBytes::Static(en_dict_prefix),
                },
            )?,
            secondary: LanguagePack::from_bytes_with_prefix_and_punctuation(
                LanguagePackDescriptor {
                    id: "uk",
                    display_name: "Ukrainian",
                    script: "Cyrillic",
                    keyboard_layout: "ukrainian-jcuken-osx",
                    punctuation_letter_keys: EMBEDDED_UKRAINIAN_PUNCTUATION_LETTER_KEYS,
                    keyboard: KeyboardMap::ukrainian_jcuken_osx(),
                },
                secondary_ngrams,
                DictionaryArtifacts {
                    dict: Cow::Borrowed(secondary_dict),
                    prefix: DictionaryPrefixBytes::Static(secondary_dict_prefix),
                },
            )?,
        })
    }

    pub fn embedded_english_artifacts() -> (&'static [u8], &'static [u8], &'static [u8]) {
        (
            include_bytes!("../data/en.ngrams.bin"),
            include_bytes!("../data/en.dict.fst"),
            include_bytes!("../data/en.dict-prefix.bin"),
        )
    }

    pub fn embedded_secondary_artifacts() -> (&'static [u8], &'static [u8], &'static [u8]) {
        (
            include_bytes!("../data/uk.ngrams.bin"),
            include_bytes!("../data/uk.dict.fst"),
            include_bytes!("../data/uk.dict-prefix.bin"),
        )
    }

    /// Loads a bundle from the artifacts produced by `typeclaw-data` in `data_dir`.
    pub fn from_data_dir(data_dir: &Path) -> Result<Self, BundleError> {
        let en_ngrams = fs::read(data_dir.join("en.ngrams.bin"))?;
        let secondary_ngrams = fs::read(data_dir.join("uk.ngrams.bin"))?;
        let en_dict = fs::read(data_dir.join("en.dict.fst"))?;
        let secondary_dict = fs::read(data_dir.join("uk.dict.fst"))?;
        let en_dict_prefix = fs::read(data_dir.join("en.dict-prefix.bin"))?;
        let secondary_dict_prefix = fs::read(data_dir.join("uk.dict-prefix.bin"))?;

        Self::from_bytes_with_prefix(
            &en_ngrams,
            &secondary_ngrams,
            en_dict,
            secondary_dict,
            &en_dict_prefix,
            &secondary_dict_prefix,
        )
    }

    /// Loads a bundle from in-memory bytes. Suitable for `include_bytes!` use from
    /// downstream crates (CLI / FFI / macOS agent).
    pub fn from_bytes<EnDict, SecondaryDict>(
        en_ngrams: &[u8],
        secondary_ngrams: &[u8],
        en_dict: EnDict,
        secondary_dict: SecondaryDict,
    ) -> Result<Self, BundleError>
    where
        EnDict: Into<Cow<'static, [u8]>>,
        SecondaryDict: Into<Cow<'static, [u8]>>,
    {
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
            secondary: LanguagePack::from_bytes_with_punctuation(
                LanguagePackDescriptor {
                    id: "uk",
                    display_name: "Ukrainian",
                    script: "Cyrillic",
                    keyboard_layout: "ukrainian-jcuken-osx",
                    punctuation_letter_keys: EMBEDDED_UKRAINIAN_PUNCTUATION_LETTER_KEYS,
                    keyboard: KeyboardMap::ukrainian_jcuken_osx(),
                },
                secondary_ngrams,
                secondary_dict,
            )?,
        })
    }

    pub fn from_bytes_with_prefix<EnDict, SecondaryDict>(
        en_ngrams: &[u8],
        secondary_ngrams: &[u8],
        en_dict: EnDict,
        secondary_dict: SecondaryDict,
        en_dict_prefix: &[u8],
        secondary_dict_prefix: &[u8],
    ) -> Result<Self, BundleError>
    where
        EnDict: Into<Cow<'static, [u8]>>,
        SecondaryDict: Into<Cow<'static, [u8]>>,
    {
        Ok(Self {
            english: LanguagePack::from_bytes_with_prefix(
                "en",
                "English",
                "Latin",
                "english-us",
                KeyboardMap::english_us(),
                en_ngrams,
                DictionaryArtifacts {
                    dict: en_dict,
                    prefix: DictionaryPrefixBytes::Borrowed(en_dict_prefix),
                },
            )?,
            secondary: LanguagePack::from_bytes_with_prefix_and_punctuation(
                LanguagePackDescriptor {
                    id: "uk",
                    display_name: "Ukrainian",
                    script: "Cyrillic",
                    keyboard_layout: "ukrainian-jcuken-osx",
                    punctuation_letter_keys: EMBEDDED_UKRAINIAN_PUNCTUATION_LETTER_KEYS,
                    keyboard: KeyboardMap::ukrainian_jcuken_osx(),
                },
                secondary_ngrams,
                DictionaryArtifacts {
                    dict: secondary_dict,
                    prefix: DictionaryPrefixBytes::Borrowed(secondary_dict_prefix),
                },
            )?,
        })
    }

    pub fn with_secondary_pack(secondary: LanguagePack) -> Result<Self, BundleError> {
        if secondary.id == "en" {
            return Err(BundleError::InvalidPack(
                "secondary pack id cannot be 'en'; English is the fixed primary side".to_owned(),
            ));
        }
        if secondary.id == "uk" {
            return Err(BundleError::InvalidPack(
                "secondary pack id cannot be 'uk'; Ukrainian is the embedded secondary side"
                    .to_owned(),
            ));
        }

        let (en_ngrams, en_dict, en_dict_prefix) = Self::embedded_english_artifacts();
        Ok(Self {
            english: LanguagePack::from_bytes_with_prefix(
                "en",
                "English",
                "Latin",
                "english-us",
                KeyboardMap::english_us(),
                en_ngrams,
                DictionaryArtifacts {
                    dict: Cow::Borrowed(en_dict),
                    prefix: DictionaryPrefixBytes::Static(en_dict_prefix),
                },
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

#[cfg(test)]
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

#[cfg(test)]
fn synthetic_pack(
    id: &str,
    display_name: &str,
    script: &str,
    keyboard_layout: &str,
    keyboard: KeyboardMap,
    words: &[(&str, u64)],
) -> LanguagePack {
    let dict = synthetic_fst(words);
    let dict_index = DictionaryIndex::from_dict(&dict);

    LanguagePack {
        id: id.to_owned(),
        display_name: display_name.to_owned(),
        script: script.to_owned(),
        keyboard_layout: keyboard_layout.to_owned(),
        punctuation_letter_keys: keyboard
            .punctuation_letter_keys_against(&KeyboardMap::english_us()),
        keyboard,
        model: synthetic_model(id, words),
        dict,
        dict_index,
        metadata: PackMetadata {
            format_version: PACK_FORMAT_VERSION,
            source_corpus: "test".to_owned(),
            source_dictionary: "test".to_owned(),
            build_id: "test".to_owned(),
            ngram_bytes: 0,
            dict_bytes: 0,
            dict_prefix_bytes: 0,
            fingerprint: 0,
        },
    }
}

#[cfg(test)]
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

    let bigram: HashMap<String, f32> = bigrams
        .into_iter()
        .map(|(key, count)| {
            let p = (count as f32 + 1.0) / (bigram_total as f32 + bigram_v);
            (key, p.log10())
        })
        .collect();

    let trigram: HashMap<String, f32> = trigrams
        .into_iter()
        .map(|(key, count)| {
            let p = (count as f32 + 1.0) / (trigram_total as f32 + trigram_v);
            (key, p.log10())
        })
        .collect();

    let mut bigram = bigram
        .into_iter()
        .filter_map(|(key, score)| bigram_key(&key).map(|key| (key, score)))
        .collect::<Vec<_>>();
    let mut trigram = trigram
        .into_iter()
        .filter_map(|(key, score)| trigram_key(&key).map(|key| (key, score)))
        .collect::<Vec<_>>();
    bigram.sort_unstable_by_key(|(key, _)| *key);
    trigram.sort_unstable_by_key(|(key, _)| *key);

    LanguageModel {
        language_tag: tag.to_owned(),
        bigram,
        trigram,
        bigram_floor,
        trigram_floor,
    }
}

#[cfg(test)]
fn synthetic_fst(words: &[(&str, u64)]) -> fst::Map<Cow<'static, [u8]>> {
    let mut sorted: Vec<(&str, u64)> = words.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut builder = fst::MapBuilder::memory();
    for (word, count) in sorted {
        builder.insert(word.as_bytes(), count).unwrap();
    }
    let bytes = builder.into_inner().unwrap();
    fst::Map::new(Cow::Owned(bytes)).unwrap()
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
    if id == "en" {
        return Err(BundleError::InvalidPack(
            "id 'en' is reserved; English is the fixed primary side".to_owned(),
        ));
    }
    if id == "uk" {
        return Err(BundleError::InvalidPack(
            "id 'uk' is reserved; Ukrainian is the embedded secondary side".to_owned(),
        ));
    }
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

fn validate_punctuation_letter_keys(value: &str) -> Result<(), BundleError> {
    let mut seen = HashSet::new();
    for character in value.chars() {
        if !seen.insert(character) {
            return Err(BundleError::InvalidPack(format!(
                "punctuation_letter_keys contains duplicate character {character:?}"
            )));
        }
        if crate::PhysicalKey::from_char(character).is_none() || character.is_ascii_alphabetic() {
            return Err(BundleError::InvalidPack(format!(
                "punctuation_letter_keys character {character:?} is not an English punctuation-position key"
            )));
        }
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

fn reject_symlink(path: &Path, label: &str) -> Result<(), BundleError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(BundleError::InvalidPack(format!(
            "{label} must not be a symlink: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn encode_compiled_language_data(
    compiled: &CompiledLanguageData,
) -> Result<Vec<u8>, NgramEncodeError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(NGRAM_MAGIC);
    push_string(&mut bytes, "language_tag", compiled.language_tag.as_str())?;
    bytes.extend_from_slice(&compiled.bigram_floor.to_le_bytes());
    bytes.extend_from_slice(&compiled.trigram_floor.to_le_bytes());
    push_entries(&mut bytes, "bigrams", &compiled.bigrams)?;
    push_entries(&mut bytes, "trigrams", &compiled.trigrams)?;
    Ok(bytes)
}

fn push_entries(
    bytes: &mut Vec<u8>,
    field: &'static str,
    entries: &[(String, f32)],
) -> Result<(), NgramEncodeError> {
    push_len(bytes, field, entries.len(), MAX_NGRAM_ENTRIES)?;
    for (key, score) in entries {
        push_string(bytes, field, key.as_str())?;
        bytes.extend_from_slice(&score.to_le_bytes());
    }
    Ok(())
}

fn push_string(
    bytes: &mut Vec<u8>,
    field: &'static str,
    value: &str,
) -> Result<(), NgramEncodeError> {
    push_len(bytes, field, value.len(), MAX_NGRAM_STRING_BYTES)?;
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn push_len(
    bytes: &mut Vec<u8>,
    field: &'static str,
    len: usize,
    max: usize,
) -> Result<(), NgramEncodeError> {
    if len > max {
        return Err(NgramEncodeError::LengthTooLarge { field, len, max });
    }
    bytes.extend_from_slice(&(len as u32).to_le_bytes());
    Ok(())
}

fn decode_compiled_language_data(bytes: &[u8]) -> Result<CompiledLanguageData, NgramDecodeError> {
    let mut reader = ArtifactReader::new(bytes);
    if reader.read_exact(NGRAM_MAGIC.len())? != NGRAM_MAGIC {
        return Err(NgramDecodeError::InvalidMagic);
    }

    let language_tag = reader.read_string("language_tag")?;
    let bigram_floor = reader.read_f32()?;
    let trigram_floor = reader.read_f32()?;
    let bigrams = reader.read_entries("bigrams")?;
    let trigrams = reader.read_entries("trigrams")?;
    reader.finish()?;

    Ok(CompiledLanguageData {
        language_tag,
        bigrams,
        trigrams,
        bigram_floor,
        trigram_floor,
    })
}

struct ArtifactReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ArtifactReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_entries(
        &mut self,
        field: &'static str,
    ) -> Result<Vec<(String, f32)>, NgramDecodeError> {
        let len = self.read_len(field, MAX_NGRAM_ENTRIES)?;
        let mut entries = Vec::with_capacity(len.min(4096));
        for _ in 0..len {
            entries.push((self.read_string(field)?, self.read_f32()?));
        }
        Ok(entries)
    }

    fn read_string(&mut self, field: &'static str) -> Result<String, NgramDecodeError> {
        let len = self.read_len(field, MAX_NGRAM_STRING_BYTES)?;
        let bytes = self.read_exact(len)?;
        Ok(str::from_utf8(bytes)
            .map_err(NgramDecodeError::InvalidUtf8)?
            .to_owned())
    }

    fn read_len(&mut self, field: &'static str, max: usize) -> Result<usize, NgramDecodeError> {
        let len = self.read_u32()? as usize;
        if len > max {
            return Err(NgramDecodeError::LengthTooLarge { field, len, max });
        }
        Ok(len)
    }

    fn read_u32(&mut self) -> Result<u32, NgramDecodeError> {
        let bytes = self.read_exact(4)?;
        let mut array = [0; 4];
        array.copy_from_slice(bytes);
        Ok(u32::from_le_bytes(array))
    }

    fn read_f32(&mut self) -> Result<f32, NgramDecodeError> {
        let bytes = self.read_exact(4)?;
        let mut array = [0; 4];
        array.copy_from_slice(bytes);
        Ok(f32::from_le_bytes(array))
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], NgramDecodeError> {
        let remaining = self.bytes.len().saturating_sub(self.position);
        if remaining < len {
            return Err(NgramDecodeError::UnexpectedEof {
                needed: len,
                remaining,
            });
        }

        let start = self.position;
        self.position += len;
        Ok(&self.bytes[start..self.position])
    }

    fn finish(self) -> Result<(), NgramDecodeError> {
        if self.position != self.bytes.len() {
            return Err(NgramDecodeError::TrailingBytes {
                read: self.position,
                total: self.bytes.len(),
            });
        }
        Ok(())
    }
}

/// Outcome of a dictionary lookup against a single language's FST.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DictLookup {
    /// Frequency of an exact match, if the token is itself a dictionary entry.
    pub exact_count: u64,
    /// Sum of frequencies for the first `prefix_sample` entries that start with the token.
    pub prefix_sum: u64,
    /// Number of dictionary entries scanned during the prefix walk (capped).
    pub prefix_sample: usize,
}

const PREFIX_SAMPLE_CAP: usize = 64;

pub fn dict_lookup<D: AsRef<[u8]>>(token: &str, dict: &fst::Map<D>) -> DictLookup {
    let mut result = DictLookup::default();

    if let Some(count) = dict.get(token.as_bytes()) {
        result.exact_count = count;
    }

    let mut range = dict.range().ge(token.as_bytes());
    let upper = prefix_upper_bound(token.as_bytes());
    if let Some(upper) = upper.as_deref() {
        range = range.lt(upper);
    }
    let mut stream = range.into_stream();
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

fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut upper = prefix.to_vec();
    while let Some(byte) = upper.pop() {
        if byte < u8::MAX {
            upper.push(byte + 1);
            return Some(upper);
        }
    }
    None
}

fn bigram_key(value: &str) -> Option<u64> {
    let mut chars = value.chars();
    let first = chars.next()?;
    let second = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(encode_bigram(first, second))
}

fn trigram_key(value: &str) -> Option<u64> {
    let mut chars = value.chars();
    let first = chars.next()?;
    let second = chars.next()?;
    let third = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(encode_trigram(first, second, third))
}

fn encode_bigram(first: char, second: char) -> u64 {
    ((first as u64) << 32) | second as u64
}

fn encode_trigram(first: char, second: char, third: char) -> u64 {
    ((first as u64) << 42) | ((second as u64) << 21) | third as u64
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
        CompiledLanguageData, DictionaryIndex, LanguagePack, LanguagePackManifest, PACK_DICT_FILE,
        PACK_DICT_PREFIX_FILE, PACK_FORMAT_VERSION, PACK_MANIFEST_FILE, PACK_NGRAMS_FILE,
        dict_lookup, encode_compiled_language_data, encode_dictionary_index,
    };

    #[test]
    fn dictionary_index_matches_capped_prefix_lookup() {
        let mut words = (0..70)
            .map(|i| (format!("a{i:02}"), i + 1))
            .collect::<Vec<_>>();
        words.extend([
            ("b".to_owned(), 1000),
            ("при".to_owned(), 7),
            ("привіт".to_owned(), 11),
        ]);
        words.sort_by(|left, right| left.0.cmp(&right.0));

        let dict = fst::Map::from_iter(words.iter().map(|(word, count)| (word.as_str(), *count)))
            .expect("test dictionary builds");
        let index = DictionaryIndex::from_dict(&dict);
        let encoded = encode_dictionary_index(&index).expect("dictionary index encodes");
        let decoded =
            DictionaryIndex::from_artifact_bytes(&encoded).expect("dictionary index decodes");

        for token in ["a", "a00", "a64", "b", "z", "п", "пр", "при", "привіт"] {
            assert_eq!(
                decoded.lookup(token, &dict),
                dict_lookup(token, &dict),
                "{token}"
            );
        }
    }

    #[test]
    fn pack_manifest_rejects_path_traversal() {
        let dir = temp_pack_dir("path-traversal");
        fs::write(
            dir.join(PACK_MANIFEST_FILE),
            r#"
format_version = 4
id = "xx"
display_name = "Bad"
script = "Latin"
layout = "english-us"
ngrams = "../ngrams.bin"
dict = "dict.fst"
dict_prefix = "dict-prefix.bin"
"#,
        )
        .unwrap();

        let error = LanguagePackManifest::read_from_dir(&dir).unwrap_err();
        assert!(error.to_string().contains("stay inside the pack directory"));
    }

    #[test]
    fn pack_manifest_rejects_unsupported_format_version() {
        let dir = temp_pack_dir("unsupported-format");
        fs::write(
            dir.join(PACK_MANIFEST_FILE),
            r#"
format_version = 999
id = "xx"
display_name = "Bad"
script = "Latin"
layout = "english-us"
ngrams = "ngrams.bin"
dict = "dict.fst"
dict_prefix = "dict-prefix.bin"
"#,
        )
        .unwrap();

        let error = LanguagePackManifest::read_from_dir(&dir).unwrap_err();
        assert!(error.to_string().contains("unsupported pack format"));
    }

    #[test]
    fn pack_manifest_rejects_reserved_ids() {
        for id in ["en", "uk"] {
            let dir = temp_pack_dir(&format!("reserved-{id}"));
            write_manifest(&dir, id);

            let error = LanguagePackManifest::read_from_dir(&dir).unwrap_err();
            assert!(error.to_string().contains("reserved"));
        }
    }

    #[test]
    fn pack_loader_rejects_malformed_ngram_bytes() {
        let dir = temp_pack_dir("bad-ngrams");
        write_manifest(&dir, "xx");
        fs::write(dir.join(PACK_NGRAMS_FILE), b"not a typeclaw ngram artifact").unwrap();
        fs::write(dir.join(PACK_DICT_FILE), valid_fst_bytes()).unwrap();
        fs::write(dir.join(PACK_DICT_PREFIX_FILE), valid_dict_prefix_bytes()).unwrap();

        let error = LanguagePack::from_pack_dir(&dir).err().unwrap();
        assert!(error.to_string().contains("ngram artifact"));
    }

    #[test]
    fn pack_loader_rejects_language_tag_mismatch() {
        let dir = temp_pack_dir("tag-mismatch");
        write_manifest(&dir, "xx");
        fs::write(dir.join(PACK_NGRAMS_FILE), valid_ngram_bytes("yy")).unwrap();
        fs::write(dir.join(PACK_DICT_FILE), valid_fst_bytes()).unwrap();
        fs::write(dir.join(PACK_DICT_PREFIX_FILE), valid_dict_prefix_bytes()).unwrap();

        let error = LanguagePack::from_pack_dir(&dir).err().unwrap();
        assert!(error.to_string().contains("language tag mismatch"));
    }

    #[test]
    fn pack_loader_rejects_malformed_fst_bytes() {
        let dir = temp_pack_dir("bad-fst");
        write_manifest(&dir, "xx");
        fs::write(dir.join(PACK_NGRAMS_FILE), valid_ngram_bytes("xx")).unwrap();
        fs::write(dir.join(PACK_DICT_FILE), b"not an fst").unwrap();
        fs::write(dir.join(PACK_DICT_PREFIX_FILE), valid_dict_prefix_bytes()).unwrap();

        let error = LanguagePack::from_pack_dir(&dir).err().unwrap();
        assert!(error.to_string().contains("fst"));
    }

    #[test]
    fn pack_loader_rejects_malformed_dict_prefix_bytes() {
        let dir = temp_pack_dir("bad-prefix");
        write_manifest(&dir, "xx");
        fs::write(dir.join(PACK_NGRAMS_FILE), valid_ngram_bytes("xx")).unwrap();
        fs::write(dir.join(PACK_DICT_FILE), valid_fst_bytes()).unwrap();
        fs::write(dir.join(PACK_DICT_PREFIX_FILE), b"not a prefix artifact").unwrap();

        let error = LanguagePack::from_pack_dir(&dir).err().unwrap();
        assert!(error.to_string().contains("dictionary prefix artifact"));
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
dict_prefix = "{PACK_DICT_PREFIX_FILE}"
"#
            ),
        )
        .unwrap();
    }

    fn valid_ngram_bytes(language_tag: &str) -> Vec<u8> {
        encode_compiled_language_data(&CompiledLanguageData {
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

    fn valid_dict_prefix_bytes() -> Vec<u8> {
        let dict = fst::Map::new(valid_fst_bytes()).unwrap();
        encode_dictionary_index(&DictionaryIndex::from_dict(&dict)).unwrap()
    }

    fn temp_pack_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "typeclaw-core-{name}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
