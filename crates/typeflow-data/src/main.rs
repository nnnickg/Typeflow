use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use fst::MapBuilder;
use serde::Deserialize;
use typeflow_core::data::{
    CompiledLanguageData, DictionaryIndex, KeyboardManifest, LanguagePack, LanguagePackManifest,
    PACK_DICT_FILE, PACK_DICT_PREFIX_FILE, PACK_FORMAT_VERSION, PACK_NGRAMS_FILE,
    encode_compiled_language_data, encode_dictionary_index,
};
use typeflow_core::{KeyboardMap, PhysicalKey};

const EN_OPUS_URL: &str = "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/mono/en.txt.gz";
const UK_OPUS_URL: &str = "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/mono/uk.txt.gz";

const EN_FREQ_URL: &str = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/en/en_full.txt";
const UK_FREQ_URL: &str = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/uk/uk_full.txt";

const EN_OPUS_INTEGRITY: DownloadIntegrity<'static> = DownloadIntegrity {
    size_bytes: Some(3_663_376_519),
    sha256: Some("1b176037f8d4af6a3d9b7aa5ab1fd567d93aff5f36a8f71d25d522a392083631"),
};
const UK_OPUS_INTEGRITY: DownloadIntegrity<'static> = DownloadIntegrity {
    size_bytes: Some(17_969_937),
    sha256: Some("d9440ca9a21d347f73a42ff13b849fb9cf247067557686575ebc7f5869988bd3"),
};
const EN_FREQ_INTEGRITY: DownloadIntegrity<'static> = DownloadIntegrity {
    size_bytes: Some(19_977_552),
    sha256: Some("7fea67ab954e2c01df6c608c9826e594cf36f8823b3243554f88245fb75dc506"),
};
const UK_FREQ_INTEGRITY: DownloadIntegrity<'static> = DownloadIntegrity {
    size_bytes: Some(5_560_013),
    sha256: Some("bfa15b3512a41771ac8ecf47f5ef8314badb76f867fb9a6afbb409265b41709f"),
};

/// EN dump is 3.66 GB compressed (~12 GB plaintext). N-gram statistics converge
/// long before that; cap the ingestion to keep iteration fast.
const EN_PLAINTEXT_BUDGET_BYTES: u64 = 200 * 1024 * 1024;
/// UK dump is small enough to process completely on build machines.
const UK_PLAINTEXT_BUDGET_BYTES: u64 = u64::MAX;

const DICT_TOP_K: usize = 500_000;
const NGRAM_BATCH_LINES: usize = 8192;
const NGRAM_BATCH_BYTES: usize = 8 * 1024 * 1024;
const TRIGRAM_CHAR_MASK: u64 = (1 << 21) - 1;

type NgramCounts = HashMap<u64, u64>;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("build-embedded") => build_embedded_artifacts(),
        Some("build-pack") => cmd_build_pack(&args[1..]),
        Some("--help") | Some("-h") | Some("help") => {
            print_usage();
            Ok(())
        }
        Some(other) => bail!("unknown command '{other}'\n\n{}", usage()),
    }
}

fn print_usage() {
    eprintln!("{}", usage());
}

fn usage() -> &'static str {
    "typeflow-data — build Typeflow language artifacts.

Usage:
  typeflow-data
      rebuild embedded EN/UK artifacts under crates/typeflow-core/data
  typeflow-data build-embedded
      same as no arguments
  typeflow-data build-pack <SPEC.toml> --out <PACK_DIR> [--cache <DIR>] [--force]
      build an installable external language pack

Pack spec fields:
  id, display_name, script, layout, alphabet, corpus, dictionary
  optional: plaintext_budget_bytes, dictionary_top_k, source_corpus,
            source_dictionary, corpus_sha256, dictionary_sha256,
            build_id, [keyboard]"
}

fn build_embedded_artifacts() -> Result<()> {
    let workspace_root = workspace_root()?;
    let cache_dir = workspace_root.join("target").join("typeflow-data-cache");
    let data_dir = workspace_root
        .join("crates")
        .join("typeflow-core")
        .join("data");
    fs::create_dir_all(&cache_dir).context("create cache dir")?;
    fs::create_dir_all(&data_dir).context("create data dir")?;

    eprintln!("=> cache:  {}", cache_dir.display());
    eprintln!("=> output: {}", data_dir.display());
    eprintln!();

    build_language(
        &cache_dir,
        &data_dir,
        EmbeddedLanguageInput {
            language: Language::English,
            opus_url: EN_OPUS_URL,
            opus_integrity: EN_OPUS_INTEGRITY,
            freq_url: EN_FREQ_URL,
            freq_integrity: EN_FREQ_INTEGRITY,
            plaintext_budget: EN_PLAINTEXT_BUDGET_BYTES,
        },
    )?;
    build_language(
        &cache_dir,
        &data_dir,
        EmbeddedLanguageInput {
            language: Language::Ukrainian,
            opus_url: UK_OPUS_URL,
            opus_integrity: UK_OPUS_INTEGRITY,
            freq_url: UK_FREQ_URL,
            freq_integrity: UK_FREQ_INTEGRITY,
            plaintext_budget: UK_PLAINTEXT_BUDGET_BYTES,
        },
    )?;

    eprintln!("\ndone.");
    Ok(())
}

#[derive(Clone, Copy)]
enum Language {
    English,
    Ukrainian,
}

#[derive(Clone, Copy)]
struct EmbeddedLanguageInput<'a> {
    language: Language,
    opus_url: &'a str,
    opus_integrity: DownloadIntegrity<'a>,
    freq_url: &'a str,
    freq_integrity: DownloadIntegrity<'a>,
    plaintext_budget: u64,
}

impl Language {
    fn tag(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Ukrainian => "uk",
        }
    }

    fn normalizer(self) -> Result<Normalizer> {
        match self {
            Language::English => Ok(Normalizer::ascii_letters()),
            Language::Ukrainian => Normalizer::from_alphabet("абвгґдеєжзиіїйклмнопрстуфхцчшщьюя")
                .context("built-in Ukrainian alphabet must be valid"),
        }
    }

    fn text_filter(self) -> TextFilter {
        match self {
            Language::English => TextFilter::None,
            Language::Ukrainian => TextFilter::UkrainianMarkers,
        }
    }
}

#[derive(Clone, Copy)]
enum TextFilter {
    None,
    UkrainianMarkers,
}

impl TextFilter {
    fn accepts(self, text: &str) -> bool {
        match self {
            TextFilter::None => true,
            TextFilter::UkrainianMarkers => text.chars().any(is_ukrainian_marker),
        }
    }
}

fn is_ukrainian_marker(character: char) -> bool {
    matches!(
        character.to_lowercase().next().unwrap_or(character),
        'і' | 'ї' | 'є' | 'ґ'
    )
}

#[derive(Clone, Debug)]
struct Normalizer {
    alphabet: HashSet<char>,
}

impl Normalizer {
    fn ascii_letters() -> Self {
        Self {
            alphabet: ('a'..='z').collect(),
        }
    }

    fn from_alphabet(alphabet: &str) -> Result<Self> {
        let mut chars = HashSet::new();
        for character in alphabet.chars() {
            let lower = character.to_lowercase().next().unwrap_or(character);
            if lower.is_whitespace() {
                continue;
            }
            chars.insert(lower);
        }
        if chars.is_empty() {
            bail!("alphabet must contain at least one non-whitespace character");
        }
        Ok(Self { alphabet: chars })
    }

    fn normalize(&self, character: char) -> Option<char> {
        let lower = character.to_lowercase().next().unwrap_or(character);
        if self.alphabet.contains(&lower) {
            Some(lower)
        } else {
            None
        }
    }

    fn alphabet_len(&self) -> usize {
        self.alphabet.len()
    }
}

#[derive(Clone, Debug, Deserialize)]
struct PackBuildSpec {
    id: String,
    display_name: String,
    script: String,
    layout: String,
    alphabet: String,
    corpus: String,
    dictionary: String,
    corpus_sha256: Option<String>,
    dictionary_sha256: Option<String>,
    plaintext_budget_bytes: Option<u64>,
    dictionary_top_k: Option<usize>,
    punctuation_letter_keys: Option<String>,
    source_corpus: Option<String>,
    source_dictionary: Option<String>,
    build_id: Option<String>,
    keyboard: Option<KeyboardManifest>,
}

struct PackBuildArgs {
    spec_path: PathBuf,
    out_dir: PathBuf,
    cache_dir: PathBuf,
    force: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct DownloadIntegrity<'a> {
    size_bytes: Option<u64>,
    sha256: Option<&'a str>,
}

fn cmd_build_pack(args: &[String]) -> Result<()> {
    let args = parse_build_pack_args(args)?;
    let spec = read_pack_spec(&args.spec_path)?;
    validate_pack_spec(&spec)?;

    let spec_dir = args.spec_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(&args.cache_dir)
        .with_context(|| format!("create cache dir {}", args.cache_dir.display()))?;
    prepare_pack_dir(&args.out_dir, args.force)?;

    eprintln!("=> spec:   {}", args.spec_path.display());
    eprintln!("=> cache:  {}", args.cache_dir.display());
    eprintln!("=> output: {}", args.out_dir.display());
    eprintln!("== {} ==", spec.id);

    let normalizer = Normalizer::from_alphabet(&spec.alphabet)?;
    let keyboard = pack_keyboard_map(spec.layout.as_str(), spec.keyboard.as_ref())?;
    let punctuation_letter_keys = spec
        .punctuation_letter_keys
        .clone()
        .unwrap_or_else(|| keyboard.punctuation_letter_keys_against(&KeyboardMap::english_us()));
    let corpus_path = resolve_input(
        spec_dir,
        &args.cache_dir,
        &spec.id,
        "corpus",
        spec.corpus.as_str(),
        spec.corpus_sha256.as_deref(),
    )?;
    let dictionary_path = resolve_input(
        spec_dir,
        &args.cache_dir,
        &spec.id,
        "dictionary",
        spec.dictionary.as_str(),
        spec.dictionary_sha256.as_deref(),
    )?;

    let plaintext_budget = spec.plaintext_budget_bytes.unwrap_or(u64::MAX);
    let (bigrams, trigrams) = count_ngrams(
        &corpus_path,
        &normalizer,
        plaintext_budget,
        TextFilter::None,
    )?;
    let compiled = compile_ngrams(
        spec.id.as_str(),
        normalizer.alphabet_len(),
        bigrams,
        trigrams,
    )?;
    let ngram_path = args.out_dir.join(PACK_NGRAMS_FILE);
    write_ngram_artifact(&ngram_path, &compiled)?;
    eprintln!(
        "   wrote {} ({} bigrams, {} trigrams)",
        ngram_path.display(),
        compiled.bigrams.len(),
        compiled.trigrams.len()
    );

    let dict_top_k = spec.dictionary_top_k.unwrap_or(DICT_TOP_K);
    let dict_path = args.out_dir.join(PACK_DICT_FILE);
    let entries = build_fst(
        &dictionary_path,
        &dict_path,
        &normalizer,
        dict_top_k,
        TextFilter::None,
    )?;
    eprintln!("   wrote {} ({} entries)", dict_path.display(), entries);
    let dict_prefix_path = args.out_dir.join(PACK_DICT_PREFIX_FILE);
    let prefix_entries = write_dict_prefix_artifact(&dict_prefix_path, &dict_path)?;
    eprintln!(
        "   wrote {} ({} prefixes)",
        dict_prefix_path.display(),
        prefix_entries
    );

    let manifest = LanguagePackManifest {
        format_version: PACK_FORMAT_VERSION,
        id: spec.id,
        display_name: spec.display_name,
        script: spec.script,
        layout: spec.layout,
        punctuation_letter_keys,
        ngrams: PathBuf::from(PACK_NGRAMS_FILE),
        dict: PathBuf::from(PACK_DICT_FILE),
        dict_prefix: PathBuf::from(PACK_DICT_PREFIX_FILE),
        source_corpus: spec.source_corpus.or(Some(spec.corpus)),
        source_dictionary: spec.source_dictionary.or(Some(spec.dictionary)),
        build_id: spec.build_id.or_else(|| Some("build-pack".to_owned())),
        keyboard: spec.keyboard,
    };
    manifest
        .write_to_dir(&args.out_dir)
        .context("write pack manifest")?;

    let pack = LanguagePack::from_pack_dir(&args.out_dir).context("validate built pack")?;
    eprintln!(
        "   validated {} ({}, fingerprint {:016x})",
        pack.id, pack.display_name, pack.metadata.fingerprint
    );
    eprintln!("\ndone.");
    Ok(())
}

fn parse_build_pack_args(args: &[String]) -> Result<PackBuildArgs> {
    if args.is_empty() {
        bail!("usage: typeflow-data build-pack <SPEC.toml> --out <PACK_DIR>");
    }

    let spec_path = PathBuf::from(&args[0]);
    let mut out_dir: Option<PathBuf> = None;
    let mut cache_dir: Option<PathBuf> = None;
    let mut force = false;

    let mut idx = 1;
    while idx < args.len() {
        match args[idx].as_str() {
            "--out" | "-o" => {
                idx += 1;
                let Some(value) = args.get(idx) else {
                    bail!("missing value after --out");
                };
                out_dir = Some(PathBuf::from(value));
            }
            "--cache" => {
                idx += 1;
                let Some(value) = args.get(idx) else {
                    bail!("missing value after --cache");
                };
                cache_dir = Some(PathBuf::from(value));
            }
            "--force" => {
                force = true;
            }
            "--help" | "-h" => {
                bail!("{}", usage());
            }
            other => bail!("unknown build-pack argument '{other}'"),
        }
        idx += 1;
    }

    let out_dir =
        out_dir.ok_or_else(|| anyhow::anyhow!("missing --out <PACK_DIR> for build-pack"))?;
    let cache_dir = match cache_dir {
        Some(path) => path,
        None => workspace_root()?
            .join("target")
            .join("typeflow-data-cache")
            .join("external-packs"),
    };

    Ok(PackBuildArgs {
        spec_path,
        out_dir,
        cache_dir,
        force,
    })
}

fn read_pack_spec(path: &Path) -> Result<PackBuildSpec> {
    let text = fs::read_to_string(path).with_context(|| format!("read spec {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parse spec {}", path.display()))
}

fn validate_pack_spec(spec: &PackBuildSpec) -> Result<()> {
    require_spec_field("id", &spec.id)?;
    validate_pack_id(&spec.id)?;
    require_spec_field("display_name", &spec.display_name)?;
    require_spec_field("script", &spec.script)?;
    require_spec_field("layout", &spec.layout)?;
    require_spec_field("alphabet", &spec.alphabet)?;
    require_spec_field("corpus", &spec.corpus)?;
    require_spec_field("dictionary", &spec.dictionary)?;

    Normalizer::from_alphabet(&spec.alphabet)?;
    if spec.plaintext_budget_bytes == Some(0) {
        bail!("plaintext_budget_bytes must be greater than zero");
    }
    if spec.dictionary_top_k == Some(0) {
        bail!("dictionary_top_k must be greater than zero");
    }
    if let Some(sha256) = spec.corpus_sha256.as_deref() {
        validate_sha256_hex("corpus_sha256", sha256)?;
    }
    if let Some(sha256) = spec.dictionary_sha256.as_deref() {
        validate_sha256_hex("dictionary_sha256", sha256)?;
    }
    if let Some(keys) = spec.punctuation_letter_keys.as_deref() {
        validate_punctuation_letter_keys(keys)?;
    }

    pack_keyboard_map(spec.layout.as_str(), spec.keyboard.as_ref())?;

    Ok(())
}

fn pack_keyboard_map(layout: &str, keyboard: Option<&KeyboardManifest>) -> Result<KeyboardMap> {
    if let Some(keyboard) = keyboard {
        return KeyboardMap::from_rows(keyboard.unshifted.as_str(), keyboard.shifted.as_str())
            .context("invalid [keyboard] rows");
    }

    KeyboardMap::named(layout).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown layout '{layout}'; set a built-in layout name or provide [keyboard] rows"
        )
    })
}

fn require_spec_field(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("spec field '{field}' must not be empty");
    }
    Ok(())
}

fn validate_pack_id(id: &str) -> Result<()> {
    if id == "en" {
        bail!("spec field 'id' cannot be 'en'; English is the fixed primary side");
    }
    if !id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("spec field 'id' must use ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

fn validate_punctuation_letter_keys(value: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for character in value.chars() {
        if !seen.insert(character) {
            bail!("punctuation_letter_keys contains duplicate character {character:?}");
        }
        if PhysicalKey::from_char(character).is_none() || character.is_ascii_alphabetic() {
            bail!(
                "punctuation_letter_keys character {character:?} is not an English punctuation-position key"
            );
        }
    }
    Ok(())
}

fn prepare_pack_dir(path: &Path, force: bool) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            bail!(
                "{} is a symlink; refusing to write pack output through symlinks",
                path.display()
            );
        }
        if !path.is_dir() {
            bail!("{} exists and is not a directory", path.display());
        }
        if !force && fs::read_dir(path)?.next().is_some() {
            bail!(
                "{} already exists and is not empty; pass --force to overwrite pack files",
                path.display()
            );
        }
        return Ok(());
    }

    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))
}

fn resolve_input(
    spec_dir: &Path,
    cache_dir: &Path,
    id: &str,
    kind: &str,
    source: &str,
    sha256: Option<&str>,
) -> Result<PathBuf> {
    if is_url(source) {
        let cache_path = cache_dir.join(cache_name_for_url(id, kind, source));
        download_with_cache(
            source,
            &cache_path,
            DownloadIntegrity {
                size_bytes: None,
                sha256,
            },
        )?;
        return Ok(cache_path);
    }

    let source_path = PathBuf::from(source);
    let path = if source_path.is_absolute() {
        source_path
    } else {
        spec_dir.join(source_path)
    };
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("stat {kind} input {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("{kind} input must not be a symlink: {}", path.display());
    }
    if !path.is_file() {
        bail!("{kind} input does not exist: {}", path.display());
    }
    verify_file_integrity(
        &path,
        DownloadIntegrity {
            size_bytes: None,
            sha256,
        },
    )?;
    Ok(path)
}

fn is_url(value: &str) -> bool {
    value.starts_with("https://") || value.starts_with("http://")
}

fn cache_name_for_url(id: &str, kind: &str, url: &str) -> String {
    let leaf = url
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(kind);
    let leaf = leaf
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{id}-{kind}-{leaf}")
}

fn build_language(
    cache_dir: &Path,
    data_dir: &Path,
    input: EmbeddedLanguageInput<'_>,
) -> Result<()> {
    let lang = input.language;
    eprintln!("== {} ==", lang.tag());
    let normalizer = lang.normalizer()?;
    let text_filter = lang.text_filter();

    let opus_cache = cache_dir.join(format!("opus-{}.txt.gz", lang.tag()));
    download_with_cache(input.opus_url, &opus_cache, input.opus_integrity)?;

    let (bigrams, trigrams) = count_ngrams(
        &opus_cache,
        &normalizer,
        input.plaintext_budget,
        text_filter,
    )?;
    let compiled = compile_ngrams(lang.tag(), normalizer.alphabet_len(), bigrams, trigrams)?;
    let ngram_path = data_dir.join(format!("{}.ngrams.bin", lang.tag()));
    write_ngram_artifact(&ngram_path, &compiled)?;
    eprintln!(
        "   wrote {} ({} bigrams, {} trigrams)",
        ngram_path.display(),
        compiled.bigrams.len(),
        compiled.trigrams.len()
    );

    let freq_cache = cache_dir.join(format!("freq-{}.txt", lang.tag()));
    download_with_cache(input.freq_url, &freq_cache, input.freq_integrity)?;
    let fst_path = data_dir.join(format!("{}.dict.fst", lang.tag()));
    let entries = build_fst(&freq_cache, &fst_path, &normalizer, DICT_TOP_K, text_filter)?;
    eprintln!("   wrote {} ({} entries)", fst_path.display(), entries);
    let prefix_path = data_dir.join(format!("{}.dict-prefix.bin", lang.tag()));
    let prefix_entries = write_dict_prefix_artifact(&prefix_path, &fst_path)?;
    eprintln!(
        "   wrote {} ({} prefixes)",
        prefix_path.display(),
        prefix_entries
    );
    eprintln!();

    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .ancestors()
        .find(|path| path.join("Cargo.lock").is_file())
        .context("could not locate workspace root")?;
    Ok(workspace.to_path_buf())
}

fn download_with_cache(url: &str, dest: &Path, integrity: DownloadIntegrity<'_>) -> Result<()> {
    if dest.is_file() {
        let size = fs::metadata(dest)?.len();
        verify_file_integrity(dest, integrity)?;
        let name = dest
            .file_name()
            .map(|value| value.to_string_lossy())
            .unwrap_or_else(|| dest.as_os_str().to_string_lossy());
        eprintln!("   cached {} ({})", name, human_bytes(size));
        return Ok(());
    }

    let temp = dest.with_extension("partial");
    let mut resume_from = prepare_partial_download(dest, &temp, integrity)?;
    if dest.is_file() {
        return Ok(());
    }
    if resume_from > 0 {
        eprintln!("   GET {} (resume from {})", url, human_bytes(resume_from));
    } else {
        eprintln!("   GET {}", url);
    }
    let started = Instant::now();
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30 * 60)))
        .build()
        .new_agent();
    let mut request = agent.get(url);
    if resume_from > 0 {
        request = request.header("Range", format!("bytes={resume_from}-"));
    }
    let mut response = request.call().with_context(|| format!("GET {url}"))?;
    let append = if resume_from > 0 && response.status().as_u16() == 206 {
        true
    } else {
        if resume_from > 0 {
            eprintln!("   server did not honor Range; restarting download");
            fs::remove_file(&temp).with_context(|| format!("remove {}", temp.display()))?;
            resume_from = 0;
        }
        false
    };
    let content_length = response.body().content_length();

    let mut reader = response.body_mut().as_reader();
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!append)
        .append(append)
        .open(&temp)
        .with_context(|| format!("open {}", temp.display()))?;
    let mut writer = io::BufWriter::new(file);
    let (written, sha256) = if append {
        copy_and_sha256_with_prefix(&temp, resume_from, &mut reader, &mut writer)?
    } else {
        copy_and_sha256(&mut reader, &mut writer)?
    };
    writer.flush()?;
    drop(writer);

    if let Some(expected) = content_length
        && written != resume_from.saturating_add(expected)
    {
        let _ = fs::remove_file(&temp);
        bail!(
            "downloaded byte count mismatch for {url}: expected Content-Length {}, wrote {}",
            resume_from.saturating_add(expected),
            written
        );
    }
    if let Some(expected) = integrity.size_bytes
        && written != expected
    {
        let _ = fs::remove_file(&temp);
        bail!(
            "downloaded byte count mismatch for {url}: expected pinned size {}, wrote {}",
            expected,
            written
        );
    }
    if let Some(expected) = integrity.sha256
        && !sha256.eq_ignore_ascii_case(expected)
    {
        let _ = fs::remove_file(&temp);
        bail!("sha256 mismatch for {url}: expected {expected}, got {sha256}");
    }

    fs::rename(&temp, dest)?;

    eprintln!(
        "   downloaded {} in {:.1}s",
        human_bytes(written),
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

fn validate_sha256_hex(field: &str, value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{field} must be a 64-character lowercase or uppercase SHA-256 hex digest");
    }
    Ok(())
}

fn verify_file_integrity(path: &Path, integrity: DownloadIntegrity<'_>) -> Result<()> {
    if integrity.size_bytes.is_none() && integrity.sha256.is_none() {
        return Ok(());
    }

    let size = fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    if let Some(expected) = integrity.size_bytes
        && size != expected
    {
        bail!(
            "cached file size mismatch for {}: expected {}, got {}",
            path.display(),
            expected,
            size
        );
    }

    if let Some(expected) = integrity.sha256 {
        let actual = sha256_file(path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            bail!(
                "cached file sha256 mismatch for {}: expected {}, got {}",
                path.display(),
                expected,
                actual
            );
        }
    }

    Ok(())
}

fn prepare_partial_download(
    dest: &Path,
    temp: &Path,
    integrity: DownloadIntegrity<'_>,
) -> Result<u64> {
    let Ok(metadata) = fs::symlink_metadata(temp) else {
        return Ok(0);
    };
    if metadata.file_type().is_symlink() {
        bail!("partial download path is a symlink: {}", temp.display());
    }
    if !metadata.is_file() {
        fs::remove_file(temp).with_context(|| format!("remove {}", temp.display()))?;
        return Ok(0);
    }

    let size = metadata.len();
    if size == 0 {
        return Ok(0);
    }
    if let Some(expected) = integrity.size_bytes {
        if size == expected {
            verify_file_integrity(temp, integrity)?;
            fs::rename(temp, dest)
                .with_context(|| format!("rename {} -> {}", temp.display(), dest.display()))?;
            eprintln!("   recovered complete partial {}", dest.display());
            return Ok(0);
        }
        if size > expected {
            fs::remove_file(temp).with_context(|| format!("remove {}", temp.display()))?;
            return Ok(0);
        }
    }

    Ok(size)
}

fn copy_and_sha256(reader: &mut impl Read, writer: &mut impl Write) -> Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut written = 0u64;
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let len = reader.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        writer.write_all(&buffer[..len])?;
        hasher.update(&buffer[..len]);
        written = written.saturating_add(len as u64);
    }

    Ok((written, hasher.finalize_hex()))
}

fn copy_and_sha256_with_prefix(
    prefix_path: &Path,
    prefix_len: u64,
    reader: &mut impl Read,
    writer: &mut impl Write,
) -> Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut written = hash_prefix(prefix_path, prefix_len, &mut hasher)?;
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let len = reader.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        writer.write_all(&buffer[..len])?;
        hasher.update(&buffer[..len]);
        written = written.saturating_add(len as u64);
    }

    Ok((written, hasher.finalize_hex()))
}

fn hash_prefix(path: &Path, prefix_len: u64, hasher: &mut Sha256) -> Result<u64> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut remaining = prefix_len;
    let mut written = 0u64;
    let mut buffer = [0u8; 64 * 1024];

    while remaining > 0 {
        let limit = buffer.len().min(remaining as usize);
        let len = file
            .read(&mut buffer[..limit])
            .with_context(|| format!("read {}", path.display()))?;
        if len == 0 {
            bail!(
                "partial download {} ended before expected resume offset {}",
                path.display(),
                prefix_len
            );
        }
        hasher.update(&buffer[..len]);
        written = written.saturating_add(len as u64);
        remaining -= len as u64;
    }

    Ok(written)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let len = file
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if len == 0 {
            break;
        }
        hasher.update(&buffer[..len]);
    }

    Ok(hasher.finalize_hex())
}

struct Sha256 {
    state: [u32; 8],
    bit_len: u64,
    buffer: [u8; 64],
    buffer_len: usize,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            bit_len: 0,
            buffer: [0; 64],
            buffer_len: 0,
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.bit_len = self.bit_len.wrapping_add((input.len() as u64) * 8);

        if self.buffer_len > 0 {
            let needed = 64 - self.buffer_len;
            let take = needed.min(input.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&input[..take]);
            self.buffer_len += take;
            input = &input[take..];

            if self.buffer_len == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffer_len = 0;
            }
        }

        while input.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&input[..64]);
            self.compress(&block);
            input = &input[64..];
        }

        if !input.is_empty() {
            self.buffer[..input.len()].copy_from_slice(input);
            self.buffer_len = input.len();
        }
    }

    fn finalize_hex(mut self) -> String {
        self.buffer[self.buffer_len] = 0x80;
        self.buffer_len += 1;

        if self.buffer_len > 56 {
            self.buffer[self.buffer_len..].fill(0);
            let block = self.buffer;
            self.compress(&block);
            self.buffer_len = 0;
        }

        self.buffer[self.buffer_len..56].fill(0);
        self.buffer[56..64].copy_from_slice(&self.bit_len.to_be_bytes());
        let block = self.buffer;
        self.compress(&block);

        let mut output = String::with_capacity(64);
        for word in self.state {
            use std::fmt::Write as _;
            let _ = write!(output, "{word:08x}");
        }
        output
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for (idx, chunk) in block.chunks_exact(4).take(16).enumerate() {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(chunk);
            w[idx] = u32::from_be_bytes(bytes);
        }
        for idx in 16..64 {
            let s0 =
                w[idx - 15].rotate_right(7) ^ w[idx - 15].rotate_right(18) ^ (w[idx - 15] >> 3);
            let s1 = w[idx - 2].rotate_right(17) ^ w[idx - 2].rotate_right(19) ^ (w[idx - 2] >> 10);
            w[idx] = w[idx - 16]
                .wrapping_add(s0)
                .wrapping_add(w[idx - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for idx in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[idx])
                .wrapping_add(w[idx]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

fn open_text_reader(path: &Path) -> Result<Box<dyn BufRead>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("gz") {
        let decoder = GzDecoder::new(BufReader::with_capacity(1 << 20, file));
        return Ok(Box::new(BufReader::with_capacity(1 << 20, decoder)));
    }

    Ok(Box::new(BufReader::with_capacity(1 << 20, file)))
}

fn count_normalized_ngrams(
    line: &str,
    normalizer: &Normalizer,
    bigrams: &mut NgramCounts,
    trigrams: &mut NgramCounts,
) {
    let mut previous_two: Option<char> = None;
    let mut previous_one: Option<char> = None;

    for character in line.chars() {
        match normalizer.normalize(character) {
            Some(letter) => {
                if let Some(first) = previous_one {
                    count_bigram(bigrams, first, letter);
                }
                if let (Some(first), Some(second)) = (previous_two, previous_one) {
                    count_trigram(trigrams, first, second, letter);
                }

                previous_two = previous_one;
                previous_one = Some(letter);
            }
            None => {
                previous_two = None;
                previous_one = None;
            }
        }
    }
}

fn count_bigram(counts: &mut NgramCounts, first: char, second: char) {
    *counts.entry(encode_bigram(first, second)).or_insert(0) += 1;
}

fn count_trigram(counts: &mut NgramCounts, first: char, second: char, third: char) {
    *counts
        .entry(encode_trigram(first, second, third))
        .or_insert(0) += 1;
}

fn count_ngram_lines(
    lines: &[String],
    normalizer: &Normalizer,
    worker_count: usize,
    text_filter: TextFilter,
) -> (NgramCounts, NgramCounts) {
    let active_workers = worker_count.clamp(1, lines.len().max(1));
    if active_workers == 1 {
        let mut bigrams = HashMap::new();
        let mut trigrams = HashMap::new();
        for line in lines {
            if !text_filter.accepts(line) {
                continue;
            }
            count_normalized_ngrams(line, normalizer, &mut bigrams, &mut trigrams);
        }
        return (bigrams, trigrams);
    }

    let chunk_size = lines.len().div_ceil(active_workers);
    let partials = thread::scope(|scope| {
        let mut handles = Vec::with_capacity(active_workers);
        for chunk in lines.chunks(chunk_size) {
            handles.push(scope.spawn(move || {
                let mut bigrams = HashMap::new();
                let mut trigrams = HashMap::new();
                for line in chunk {
                    if !text_filter.accepts(line) {
                        continue;
                    }
                    count_normalized_ngrams(line, normalizer, &mut bigrams, &mut trigrams);
                }
                (bigrams, trigrams)
            }));
        }

        let mut partials = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.join() {
                Ok(counts) => partials.push(counts),
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
        partials
    });

    let mut bigrams = HashMap::new();
    let mut trigrams = HashMap::new();
    for (batch_bigrams, batch_trigrams) in partials {
        merge_ngram_counts(&mut bigrams, batch_bigrams);
        merge_ngram_counts(&mut trigrams, batch_trigrams);
    }
    (bigrams, trigrams)
}

fn merge_ngram_counts(target: &mut NgramCounts, source: NgramCounts) {
    for (key, count) in source {
        *target.entry(key).or_insert(0) += count;
    }
}

fn merge_ngram_batch(
    batch: &mut Vec<String>,
    normalizer: &Normalizer,
    worker_count: usize,
    text_filter: TextFilter,
    bigrams: &mut NgramCounts,
    trigrams: &mut NgramCounts,
) {
    if batch.is_empty() {
        return;
    }

    let (batch_bigrams, batch_trigrams) =
        count_ngram_lines(batch, normalizer, worker_count, text_filter);
    merge_ngram_counts(bigrams, batch_bigrams);
    merge_ngram_counts(trigrams, batch_trigrams);
    batch.clear();
}

fn count_ngrams(
    corpus_path: &Path,
    normalizer: &Normalizer,
    plaintext_budget: u64,
    text_filter: TextFilter,
) -> Result<(NgramCounts, NgramCounts)> {
    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);

    eprintln!(
        "   counting n-grams (budget {}, workers {})...",
        human_bytes(plaintext_budget),
        worker_count
    );

    let started = Instant::now();
    let reader = open_text_reader(corpus_path)?;

    let mut bigrams: NgramCounts = HashMap::new();
    let mut trigrams: NgramCounts = HashMap::new();
    let mut bytes_seen: u64 = 0;
    let mut last_progress = Instant::now();
    let mut batch: Vec<String> = Vec::with_capacity(NGRAM_BATCH_LINES);
    let mut batch_bytes: usize = 0;

    for line_result in reader.lines() {
        let line = line_result?;
        bytes_seen = bytes_seen.saturating_add(line.len() as u64 + 1);
        batch_bytes = batch_bytes.saturating_add(line.len() + 1);
        batch.push(line);

        if batch.len() >= NGRAM_BATCH_LINES
            || batch_bytes >= NGRAM_BATCH_BYTES
            || bytes_seen >= plaintext_budget
        {
            merge_ngram_batch(
                &mut batch,
                normalizer,
                worker_count,
                text_filter,
                &mut bigrams,
                &mut trigrams,
            );
            batch_bytes = 0;
        }

        if last_progress.elapsed().as_secs() >= 5 {
            eprintln!(
                "      {} processed | bigrams={} trigrams={}",
                human_bytes(bytes_seen),
                bigrams.len(),
                trigrams.len()
            );
            last_progress = Instant::now();
        }

        if bytes_seen >= plaintext_budget {
            break;
        }
    }

    merge_ngram_batch(
        &mut batch,
        normalizer,
        worker_count,
        text_filter,
        &mut bigrams,
        &mut trigrams,
    );

    eprintln!(
        "      done in {:.1}s | {} processed",
        started.elapsed().as_secs_f32(),
        human_bytes(bytes_seen)
    );

    Ok((bigrams, trigrams))
}

fn compile_ngrams(
    language_tag: &str,
    alphabet_len: usize,
    bigrams: NgramCounts,
    trigrams: NgramCounts,
) -> Result<CompiledLanguageData> {
    if bigrams.is_empty() {
        bail!("corpus produced no bigrams; check alphabet/corpus");
    }
    if trigrams.is_empty() {
        bail!("corpus produced no trigrams; check alphabet/corpus");
    }

    let bigram_total: u64 = bigrams.values().sum();
    let trigram_total: u64 = trigrams.values().sum();

    let bigram_v = ngram_vocabulary_size(alphabet_len, 2)?;
    let trigram_v = ngram_vocabulary_size(alphabet_len, 3)?;

    let bigram_floor = ((1.0_f32) / (bigram_total as f32 + bigram_v)).log10();
    let trigram_floor = ((1.0_f32) / (trigram_total as f32 + trigram_v)).log10();

    let mut bigrams_vec: Vec<(String, f32)> = bigrams
        .into_iter()
        .map(|(key, count)| {
            let prob = (count as f32 + 1.0) / (bigram_total as f32 + bigram_v);
            (decode_bigram(key), prob.log10())
        })
        .collect();
    bigrams_vec.sort_by(|a, b| a.0.cmp(&b.0));

    let mut trigrams_vec: Vec<(String, f32)> = trigrams
        .into_iter()
        .map(|(key, count)| {
            let prob = (count as f32 + 1.0) / (trigram_total as f32 + trigram_v);
            (decode_trigram(key), prob.log10())
        })
        .collect();
    trigrams_vec.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(CompiledLanguageData {
        language_tag: language_tag.to_owned(),
        bigrams: bigrams_vec,
        trigrams: trigrams_vec,
        bigram_floor,
        trigram_floor,
    })
}

fn ngram_vocabulary_size(alphabet_len: usize, order: u32) -> Result<f32> {
    let size = alphabet_len
        .max(1)
        .checked_pow(order)
        .context("ngram smoothing vocabulary is too large")?;
    Ok(size as f32)
}

fn encode_bigram(first: char, second: char) -> u64 {
    ((first as u64) << 32) | second as u64
}

fn encode_trigram(first: char, second: char, third: char) -> u64 {
    ((first as u64) << 42) | ((second as u64) << 21) | third as u64
}

fn decode_bigram(key: u64) -> String {
    let mut out = String::with_capacity(8);
    push_scalar(&mut out, key >> 32);
    push_scalar(&mut out, key & u32::MAX as u64);
    out
}

fn decode_trigram(key: u64) -> String {
    let mut out = String::with_capacity(12);
    push_scalar(&mut out, key >> 42);
    push_scalar(&mut out, (key >> 21) & TRIGRAM_CHAR_MASK);
    push_scalar(&mut out, key & TRIGRAM_CHAR_MASK);
    out
}

fn push_scalar(out: &mut String, value: u64) {
    if let Some(character) = char::from_u32(value as u32) {
        out.push(character);
    }
}

fn write_ngram_artifact(path: &Path, value: &CompiledLanguageData) -> Result<()> {
    let bytes = encode_compiled_language_data(value)?;
    fs::write(path, bytes)?;
    Ok(())
}

fn write_dict_prefix_artifact(path: &Path, fst_path: &Path) -> Result<usize> {
    let dict_bytes = fs::read(fst_path).with_context(|| format!("read {}", fst_path.display()))?;
    let dict = fst::Map::new(dict_bytes).with_context(|| format!("load {}", fst_path.display()))?;
    let index = DictionaryIndex::from_dict(&dict);
    let bytes = encode_dictionary_index(&index)?;
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(index.len())
}

fn build_fst(
    freq_path: &Path,
    fst_path: &Path,
    normalizer: &Normalizer,
    dict_top_k: usize,
    text_filter: TextFilter,
) -> Result<usize> {
    eprintln!("   building dictionary FST...");

    let reader = open_text_reader(freq_path)?;

    let mut counts: HashMap<String, u64> = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        let mut parts = line.split_whitespace();
        let Some(word) = parts.next() else {
            continue;
        };
        let Some(count_str) = parts.next() else {
            continue;
        };
        let Ok(count) = count_str.parse::<u64>() else {
            continue;
        };

        let normalized_word: String = word
            .chars()
            .map(|character| normalizer.normalize(character))
            .collect::<Option<String>>()
            .unwrap_or_default();
        if normalized_word.is_empty() {
            continue;
        }
        if !text_filter.accepts(&normalized_word) {
            continue;
        }

        *counts.entry(normalized_word).or_insert(0) += count;
    }

    let mut entries: Vec<(String, u64)> = counts.into_iter().collect();
    if entries.is_empty() {
        bail!("frequency list at {} is empty", freq_path.display());
    }

    // Take the most-frequent K, then sort lexicographically for the FST builder.
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    entries.truncate(dict_top_k);
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let writer = io::BufWriter::new(File::create(fst_path)?);
    let mut builder = MapBuilder::new(writer)?;
    for (word, count) in &entries {
        builder.insert(word.as_bytes(), *count)?;
    }
    builder.finish()?;

    Ok(entries.len())
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == u64::MAX {
        return "unbounded".to_owned();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        NgramCounts, Normalizer, Sha256, TextFilter, compile_ngrams, count_ngram_lines,
        count_normalized_ngrams, encode_bigram, encode_compiled_language_data, encode_trigram,
        validate_pack_id,
    };

    #[test]
    fn sha256_matches_known_vectors() {
        let mut empty = Sha256::new();
        empty.update(b"");
        assert_eq!(
            empty.finalize_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let mut abc = Sha256::new();
        abc.update(b"abc");
        assert_eq!(
            abc.finalize_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn normalized_ngram_counting_lowercases_and_resets_on_non_letters() {
        let normalizer = Normalizer::ascii_letters();
        let mut bigrams = HashMap::new();
        let mut trigrams = HashMap::new();

        count_normalized_ngrams("ABc de", &normalizer, &mut bigrams, &mut trigrams);

        assert_eq!(bigram_count(&bigrams, 'a', 'b'), Some(&1));
        assert_eq!(bigram_count(&bigrams, 'b', 'c'), Some(&1));
        assert_eq!(bigram_count(&bigrams, 'd', 'e'), Some(&1));
        assert_eq!(bigram_count(&bigrams, 'c', 'd'), None);
        assert_eq!(trigram_count(&trigrams, 'a', 'b', 'c'), Some(&1));
        assert_eq!(trigram_count(&trigrams, 'b', 'c', 'd'), None);
    }

    #[test]
    fn line_batch_counting_merges_worker_results() {
        let normalizer = Normalizer::ascii_letters();
        let lines = vec!["abc".to_owned(), "bcd".to_owned()];

        let (bigrams, trigrams) = count_ngram_lines(&lines, &normalizer, 2, TextFilter::None);

        assert_eq!(bigram_count(&bigrams, 'a', 'b'), Some(&1));
        assert_eq!(bigram_count(&bigrams, 'b', 'c'), Some(&2));
        assert_eq!(bigram_count(&bigrams, 'c', 'd'), Some(&1));
        assert_eq!(trigram_count(&trigrams, 'a', 'b', 'c'), Some(&1));
        assert_eq!(trigram_count(&trigrams, 'b', 'c', 'd'), Some(&1));
    }

    #[test]
    fn compile_ngrams_smooths_against_full_alphabet_vocabulary() -> super::Result<()> {
        let mut bigrams = HashMap::new();
        bigrams.insert(encode_bigram('a', 'b'), 2);
        let mut trigrams = HashMap::new();
        trigrams.insert(encode_trigram('a', 'b', 'c'), 3);

        let compiled = compile_ngrams("xx", 3, bigrams, trigrams)?;

        let expected_bigram_floor = (1.0_f32 / (2.0 + 9.0)).log10();
        let expected_trigram_floor = (1.0_f32 / (3.0 + 27.0)).log10();
        assert!((compiled.bigram_floor - expected_bigram_floor).abs() < f32::EPSILON);
        assert!((compiled.trigram_floor - expected_trigram_floor).abs() < f32::EPSILON);
        Ok(())
    }

    #[test]
    fn compile_ngrams_serializes_deterministically() -> super::Result<()> {
        let bigram_items = [
            (encode_bigram('b', 'a'), 4),
            (encode_bigram('a', 'b'), 2),
            (encode_bigram('c', 'a'), 1),
        ];
        let trigram_items = [
            (encode_trigram('c', 'a', 'b'), 7),
            (encode_trigram('a', 'b', 'c'), 3),
            (encode_trigram('b', 'c', 'a'), 5),
        ];

        let first = compile_ngrams(
            "xx",
            3,
            bigram_items.into_iter().collect(),
            trigram_items.into_iter().collect(),
        )?;
        let second = compile_ngrams(
            "xx",
            3,
            bigram_items.into_iter().rev().collect(),
            trigram_items.into_iter().rev().collect(),
        )?;

        assert_eq!(
            encode_compiled_language_data(&first)?,
            encode_compiled_language_data(&second)?
        );
        Ok(())
    }

    #[test]
    fn pack_id_rejects_reserved_and_path_like_values() {
        assert!(validate_pack_id("uk").is_ok());
        assert!(validate_pack_id("en").is_err());
        assert!(validate_pack_id("../uk").is_err());
        assert!(validate_pack_id("uk/en").is_err());
    }

    #[test]
    fn ukrainian_marker_filter_rejects_unmarked_cyrillic_text() {
        assert!(TextFilter::UkrainianMarkers.accepts("привіт"));
        assert!(TextFilter::UkrainianMarkers.accepts("ґрунт"));
        assert!(!TextFilter::UkrainianMarkers.accepts("абвд"));
        assert!(!TextFilter::UkrainianMarkers.accepts("прст"));
    }

    fn bigram_count(counts: &NgramCounts, first: char, second: char) -> Option<&u64> {
        counts.get(&encode_bigram(first, second))
    }

    fn trigram_count(counts: &NgramCounts, first: char, second: char, third: char) -> Option<&u64> {
        counts.get(&encode_trigram(first, second, third))
    }
}
