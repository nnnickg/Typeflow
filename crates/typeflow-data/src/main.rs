use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use fst::MapBuilder;
use serde::Deserialize;
use typeflow_core::KeyboardMap;
use typeflow_core::data::{
    CompiledLanguageData, KeyboardManifest, LanguagePack, LanguagePackManifest, PACK_DICT_FILE,
    PACK_FORMAT_VERSION, PACK_NGRAMS_FILE, encode_compiled_language_data,
};

const EN_OPUS_URL: &str = "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/mono/en.txt.gz";
const UK_OPUS_URL: &str = "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/mono/uk.txt.gz";

const EN_FREQ_URL: &str = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/en/en_full.txt";
const UK_FREQ_URL: &str = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/uk/uk_full.txt";

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
            source_dictionary, build_id, [keyboard]"
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
        Language::English,
        EN_OPUS_URL,
        EN_FREQ_URL,
        EN_PLAINTEXT_BUDGET_BYTES,
    )?;
    build_language(
        &cache_dir,
        &data_dir,
        Language::Ukrainian,
        UK_OPUS_URL,
        UK_FREQ_URL,
        UK_PLAINTEXT_BUDGET_BYTES,
    )?;

    eprintln!("\ndone.");
    Ok(())
}

#[derive(Clone, Copy)]
enum Language {
    English,
    Ukrainian,
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
    )?;
    let dictionary_path = resolve_input(
        spec_dir,
        &args.cache_dir,
        &spec.id,
        "dictionary",
        spec.dictionary.as_str(),
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

    let manifest = LanguagePackManifest {
        format_version: PACK_FORMAT_VERSION,
        id: spec.id,
        display_name: spec.display_name,
        script: spec.script,
        layout: spec.layout,
        ngrams: PathBuf::from(PACK_NGRAMS_FILE),
        dict: PathBuf::from(PACK_DICT_FILE),
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
) -> Result<PathBuf> {
    if is_url(source) {
        let cache_path = cache_dir.join(cache_name_for_url(id, kind, source));
        download_with_cache(source, &cache_path)?;
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
    lang: Language,
    opus_url: &str,
    freq_url: &str,
    plaintext_budget: u64,
) -> Result<()> {
    eprintln!("== {} ==", lang.tag());
    let normalizer = lang.normalizer()?;

    let opus_cache = cache_dir.join(format!("opus-{}.txt.gz", lang.tag()));
    download_with_cache(opus_url, &opus_cache)?;

    let (bigrams, trigrams) = count_ngrams(&opus_cache, &normalizer, plaintext_budget)?;
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
    download_with_cache(freq_url, &freq_cache)?;
    let fst_path = data_dir.join(format!("{}.dict.fst", lang.tag()));
    let entries = build_fst(&freq_cache, &fst_path, &normalizer, DICT_TOP_K)?;
    eprintln!("   wrote {} ({} entries)", fst_path.display(), entries);
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

fn download_with_cache(url: &str, dest: &Path) -> Result<()> {
    if dest.is_file() {
        let size = fs::metadata(dest)?.len();
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

    let mut reader = response.body_mut().as_reader();
    let temp = dest.with_extension("partial");
    let mut writer = io::BufWriter::new(File::create(&temp)?);
    let written = io::copy(&mut reader, &mut writer)?;
    writer.flush()?;
    drop(writer);
    fs::rename(&temp, dest)?;

    eprintln!(
        "   downloaded {} in {:.1}s",
        human_bytes(written),
        started.elapsed().as_secs_f32()
    );
    Ok(())
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
    entries.sort_by_key(|entry| Reverse(entry.1));
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
