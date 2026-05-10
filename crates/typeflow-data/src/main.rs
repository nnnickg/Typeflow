use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use fst::MapBuilder;
use serde::Deserialize;
use typeflow_core::KeyboardMap;
use typeflow_core::data::{
    CompiledLanguageData, DictionaryIndex, KeyboardManifest, LanguagePack, LanguagePackManifest,
    PACK_DICT_FILE, PACK_DICT_PREFIX_FILE, PACK_FORMAT_VERSION, PACK_NGRAMS_FILE,
    encode_compiled_language_data, encode_dictionary_index,
};

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
    let (bigrams, trigrams) = count_ngrams(&corpus_path, &normalizer, plaintext_budget)?;
    let compiled = compile_ngrams(spec.id.as_str(), bigrams, trigrams)?;
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
    let entries = build_fst(&dictionary_path, &dict_path, &normalizer, dict_top_k)?;
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

    if let Some(keyboard) = &spec.keyboard {
        KeyboardMap::from_rows(keyboard.unshifted.as_str(), keyboard.shifted.as_str())
            .context("invalid [keyboard] rows")?;
    } else if KeyboardMap::named(&spec.layout).is_none() {
        bail!(
            "unknown layout '{}'; set a built-in layout name or provide [keyboard] rows",
            spec.layout
        );
    }

    Ok(())
}

fn require_spec_field(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("spec field '{field}' must not be empty");
    }
    Ok(())
}

fn prepare_pack_dir(path: &Path, force: bool) -> Result<()> {
    if path.exists() {
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

    let opus_cache = cache_dir.join(format!("opus-{}.txt.gz", lang.tag()));
    download_with_cache(input.opus_url, &opus_cache, input.opus_integrity)?;

    let (bigrams, trigrams) = count_ngrams(&opus_cache, &normalizer, input.plaintext_budget)?;
    let compiled = compile_ngrams(lang.tag(), bigrams, trigrams)?;
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
    let entries = build_fst(&freq_cache, &fst_path, &normalizer, DICT_TOP_K)?;
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

    eprintln!("   GET {}", url);
    let started = Instant::now();
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30 * 60)))
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let content_length = response.body().content_length();

    let mut reader = response.body_mut().as_reader();
    let temp = dest.with_extension("partial");
    let mut writer = io::BufWriter::new(File::create(&temp)?);
    let (written, sha256) = copy_and_sha256(&mut reader, &mut writer)?;
    writer.flush()?;
    drop(writer);

    if let Some(expected) = content_length
        && written != expected
    {
        let _ = fs::remove_file(&temp);
        bail!(
            "downloaded byte count mismatch for {url}: expected Content-Length {}, wrote {}",
            expected,
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

fn count_ngrams(
    corpus_path: &Path,
    normalizer: &Normalizer,
    plaintext_budget: u64,
) -> Result<(HashMap<String, u64>, HashMap<String, u64>)> {
    eprintln!(
        "   counting n-grams (budget {})...",
        human_bytes(plaintext_budget)
    );

    let started = Instant::now();
    let reader = open_text_reader(corpus_path)?;

    let mut bigrams: HashMap<String, u64> = HashMap::new();
    let mut trigrams: HashMap<String, u64> = HashMap::new();
    let mut bytes_seen: u64 = 0;
    let mut last_progress = Instant::now();
    let mut window: Vec<char> = Vec::with_capacity(3);

    for line_result in reader.lines() {
        let line = line_result?;
        bytes_seen = bytes_seen.saturating_add(line.len() as u64 + 1);

        for character in line.chars() {
            match normalizer.normalize(character) {
                Some(letter) => {
                    window.push(letter);
                    if window.len() > 3 {
                        window.remove(0);
                    }
                    if window.len() >= 2 {
                        let bigram: String = window[window.len() - 2..].iter().collect();
                        *bigrams.entry(bigram).or_insert(0) += 1;
                    }
                    if window.len() == 3 {
                        let trigram: String = window.iter().collect();
                        *trigrams.entry(trigram).or_insert(0) += 1;
                    }
                }
                None => window.clear(),
            }
        }
        window.clear();

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

    eprintln!(
        "      done in {:.1}s | {} processed",
        started.elapsed().as_secs_f32(),
        human_bytes(bytes_seen)
    );

    Ok((bigrams, trigrams))
}

fn compile_ngrams(
    language_tag: &str,
    bigrams: HashMap<String, u64>,
    trigrams: HashMap<String, u64>,
) -> Result<CompiledLanguageData> {
    if bigrams.is_empty() {
        bail!("corpus produced no bigrams; check alphabet/corpus");
    }
    if trigrams.is_empty() {
        bail!("corpus produced no trigrams; check alphabet/corpus");
    }

    let bigram_total: u64 = bigrams.values().sum();
    let trigram_total: u64 = trigrams.values().sum();

    // Laplace-style add-one smoothing across the observed vocabulary.
    let bigram_v = bigrams.len().max(1) as f32;
    let trigram_v = trigrams.len().max(1) as f32;

    let bigram_floor = ((1.0_f32) / (bigram_total as f32 + bigram_v)).log10();
    let trigram_floor = ((1.0_f32) / (trigram_total as f32 + trigram_v)).log10();

    let mut bigrams_vec: Vec<(String, f32)> = bigrams
        .into_iter()
        .map(|(key, count)| {
            let prob = (count as f32 + 1.0) / (bigram_total as f32 + bigram_v);
            (key, prob.log10())
        })
        .collect();
    bigrams_vec.sort_by(|a, b| a.0.cmp(&b.0));

    let mut trigrams_vec: Vec<(String, f32)> = trigrams
        .into_iter()
        .map(|(key, count)| {
            let prob = (count as f32 + 1.0) / (trigram_total as f32 + trigram_v);
            (key, prob.log10())
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
    use super::Sha256;

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
}
