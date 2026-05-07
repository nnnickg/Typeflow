mod config;

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use crossterm::{
    ExecutableCommand, cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{self, ClearType},
};
use fst::Streamer;
use typeflow_core::Layout;
use typeflow_core::data::{
    LanguageBundle, LanguagePack, LanguagePackManifest, PACK_DICT_FILE, PACK_MANIFEST_FILE,
    PACK_NGRAMS_FILE,
};
use typeflow_core::{
    Action, Decision, Engine, EngineConfig, InputEvent, LayoutScore, ScoreAnalysis,
    has_dictionary_evidence,
};

use config::{Config, ConfigSource};

struct CliArgs {
    subcommand: Option<String>,
    rest: Vec<String>,
    config_path: Option<PathBuf>,
}

fn main() -> ExitCode {
    let parsed = match parse_args(env::args().skip(1).collect()) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}\n\n{}", usage());
            return ExitCode::from(2);
        }
    };

    let result = match parsed.subcommand.as_deref() {
        Some("type") => cmd_type(&parsed.rest, parsed.config_path.as_deref()),
        Some("stream") => cmd_stream(&parsed.rest, parsed.config_path.as_deref()),
        Some("repl") => cmd_repl(parsed.config_path.as_deref()),
        Some("predict") => cmd_predict(&parsed.rest, parsed.config_path.as_deref()),
        Some("convert") => cmd_convert(&parsed.rest, parsed.config_path.as_deref()),
        Some("eval") => cmd_eval(&parsed.rest, parsed.config_path.as_deref()),
        Some("bench") => cmd_bench(&parsed.rest, parsed.config_path.as_deref()),
        Some("model") => cmd_model(&parsed.rest, parsed.config_path.as_deref()),
        Some("pack") => cmd_pack(&parsed.rest, parsed.config_path.as_deref()),
        Some("config") => cmd_config(&parsed.rest, parsed.config_path.as_deref()),
        Some("--help") | Some("-h") | None => {
            print_usage();
            return ExitCode::from(0);
        }
        Some(other) => Err(format!("unknown subcommand: {other}\n\n{}", usage())),
    };

    match result {
        Ok(()) => ExitCode::from(0),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn parse_args(raw: Vec<String>) -> Result<CliArgs, String> {
    let mut subcommand: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut config_path: Option<PathBuf> = None;

    let mut iter = raw.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            let value = iter
                .next()
                .ok_or_else(|| "missing path after --config".to_owned())?;
            config_path = Some(PathBuf::from(value));
        } else if let Some(value) = arg.strip_prefix("--config=") {
            config_path = Some(PathBuf::from(value));
        } else if subcommand.is_none() {
            subcommand = Some(arg);
        } else {
            rest.push(arg);
        }
    }

    Ok(CliArgs {
        subcommand,
        rest,
        config_path,
    })
}

fn print_usage() {
    eprintln!("{}", usage());
}

fn usage() -> &'static str {
    "typeflow — feel the engine before macOS exists.

Usage:
  typeflow [--config <path>] type <KEYS> [<KEYS>...]
                                 process one or more tokens, print per-key trace + final score
  typeflow [--config <path>] stream
                                 read stdin token-per-line, output TSV decisions
  typeflow [--config <path>] repl
                                 interactive raw-mode TTY; type live, see scores update
  typeflow [--config <path>] predict [--json] <KEYS>
                                 one-shot: print the picked layout + rendered text. Pipeline-friendly.
  typeflow [--config <path>] convert <KEYS>
                                 force-convert one token to the opposite layout
  typeflow [--config <path>] eval [--generated [limit-per-layout] | <tsv>]
                                 run labeled checks. TSV: keys<TAB>expected-layout
  typeflow [--config <path>] bench [iterations]
                                 micro-benchmark the hot engine loop
  typeflow [--config <path>] model
                                 print embedded/loaded language-pack metadata
  typeflow [--config <path>] pack export-ru <DIR>
                                 export the embedded Russian model as an installable pack
  typeflow [--config <path>] pack install <PACK_DIR>
                                 validate and install a language pack
  typeflow [--config <path>] pack list
                                 list installed language packs
  typeflow [--config <path>] pack use <ID>
                                 set the active secondary language in config
  typeflow [--config <path>] pack inspect <PACK_DIR|ID>
                                 print pack manifest/model metadata
  typeflow config init [<path>]  write a fully-commented default config to <path>
                                 (defaults to ~/.config/typeflow/config.toml)
  typeflow config show           print the resolved effective config (after merging sources)

Flags:
  --config <path>                use this TOML file instead of the default search path

Environment:
  TYPEFLOW_DATA_DIR              override the language-bundle directory
  TYPEFLOW_PACK_DIR              override the installed pack directory
  TYPEFLOW_CONFIG                path to config TOML (overridden by --config)"
}

fn load_config(explicit: Option<&Path>) -> Result<ConfigSource, String> {
    Config::load(explicit)
}

fn configured_data_dir(config: &Config) -> Option<PathBuf> {
    if let Ok(env_path) = env::var("TYPEFLOW_DATA_DIR") {
        return Some(PathBuf::from(env_path));
    }
    config.data.directory.clone()
}

fn configured_pack_dir(config: &Config) -> Option<PathBuf> {
    if let Ok(env_path) = env::var("TYPEFLOW_PACK_DIR") {
        return Some(PathBuf::from(env_path));
    }
    config
        .packs
        .directory
        .clone()
        .or_else(config::default_pack_dir)
}

fn build_engine(config: &Config) -> Result<Engine, String> {
    let bundle = load_language_bundle(config)?;
    let engine_config: EngineConfig = config.engine.into();
    Ok(Engine::new(engine_config, bundle))
}

fn load_language_bundle(config: &Config) -> Result<LanguageBundle, String> {
    if let Some(dir) = configured_data_dir(config) {
        return LanguageBundle::from_data_dir(&dir)
            .map_err(|e| format!("load bundle from {}: {e}", dir.display()));
    }

    let secondary_id = normalized_secondary_id(config);
    if let Some(pack_dir) = configured_pack_dir(config) {
        let candidate = pack_dir.join(&secondary_id);
        if candidate.join(PACK_MANIFEST_FILE).is_file() {
            return LanguageBundle::from_secondary_pack_dir(&candidate)
                .map_err(|e| format!("load pack {}: {e}", candidate.display()));
        }
    }

    if secondary_id == "ru" {
        return LanguageBundle::embedded().map_err(|e| format!("load embedded bundle: {e}"));
    }

    let pack_dir = configured_pack_dir(config)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unknown: HOME is unset>".to_owned());
    Err(format!(
        "language pack '{secondary_id}' is not installed in {pack_dir}; run `typeflow pack install <PACK_DIR>`"
    ))
}

fn normalized_secondary_id(config: &Config) -> String {
    let id = config.language.secondary.trim();
    if id.is_empty() {
        "ru".to_owned()
    } else {
        id.to_owned()
    }
}

fn cmd_type(args: &[String], explicit_config: Option<&Path>) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: typeflow type <KEYS> [<KEYS>...]".into());
    }
    let source = load_config(explicit_config)?;
    let mut engine = build_engine(&source.config)?;

    for (idx, token) in args.iter().enumerate() {
        if idx > 0 {
            engine.process(InputEvent::EndToken);
            println!("--- end of token ---");
        }
        process_token_verbose(&mut engine, token)?;
    }

    let score = engine.token_score();
    println!();
    println!("FINAL");
    println!("layout: {}", layout_label(&engine, engine.current_layout()));
    print_score_lines(&engine, &score);
    print_margin_line(&engine, &score, engine.config());
    Ok(())
}

fn process_token_verbose(engine: &mut Engine, token: &str) -> Result<(), String> {
    for character in token.chars() {
        let input = input_event_for_char(engine, character);
        let output = engine.process(input);
        println!(
            "key='{character}' {}='{}' {}='{}' decision={} action={}",
            token_label(engine, Layout::English),
            output.candidates.english,
            token_label(engine, Layout::Secondary),
            output.candidates.secondary,
            decision_label(engine, output.decision),
            action_label(engine, &output.action)
        );
    }
    Ok(())
}

fn cmd_stream(args: &[String], explicit_config: Option<&Path>) -> Result<(), String> {
    if !args.is_empty() {
        return Err("usage: typeflow stream  (reads tokens from stdin, one per line)".into());
    }
    let source = load_config(explicit_config)?;
    let mut engine = build_engine(&source.config)?;

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    if stdin.is_terminal() {
        eprintln!("typeflow stream: reading tokens from stdin (one per line); Ctrl-D to end.");
    }

    writeln!(
        stdout,
        "# keys\tdecision\trendered\t{}_total\t{}_total\tmargin",
        token_label(&engine, Layout::English),
        token_label(&engine, Layout::Secondary)
    )
    .map_err(io_str)?;

    for line in stdin.lock().lines() {
        let line = line.map_err(io_str)?;
        let token = line.trim();
        if token.is_empty() {
            continue;
        }

        engine.reset_layout(Layout::English);
        let rendered = match run_token_committed(&mut engine, token) {
            Ok(rendered) => rendered,
            Err(error) => {
                writeln!(stdout, "{token}\tERROR\t{error}\t0\t0\t0").map_err(io_str)?;
                continue;
            }
        };

        let score = engine.token_score();
        let layout = engine.current_layout();
        let margin = score.english.total - score.secondary.total;
        writeln!(
            stdout,
            "{token}\t{}\t{}\t{:.2}\t{:.2}\t{:+.2}",
            layout_label(&engine, layout),
            rendered,
            score.english.total,
            score.secondary.total,
            margin,
        )
        .map_err(io_str)?;
    }

    Ok(())
}

fn run_token_silent(engine: &mut Engine, token: &str) -> Result<(), String> {
    run_token_committed(engine, token).map(|_| ())
}

fn run_token_committed(engine: &mut Engine, token: &str) -> Result<String, String> {
    let mut committed = String::new();
    for character in token.chars() {
        let input = input_event_for_char(engine, character);
        let action = engine.process_action(input);
        apply_action(&action, &mut committed);
    }
    Ok(committed)
}

fn input_event_for_char(engine: &Engine, character: char) -> InputEvent {
    engine.input_event_from_char(character)
}

fn cmd_predict(args: &[String], explicit_config: Option<&Path>) -> Result<(), String> {
    let mut json = false;
    let mut tokens: Vec<&String> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            other if other.starts_with("--") => {
                return Err(format!("unknown flag for predict: {other}"));
            }
            _ => tokens.push(arg),
        }
    }
    if tokens.is_empty() {
        return Err("usage: typeflow predict [--json] <KEYS>".into());
    }

    let source = load_config(explicit_config)?;
    let mut engine = build_engine(&source.config)?;

    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    for (idx, token) in tokens.iter().enumerate() {
        if idx > 0 {
            engine.reset_layout(Layout::English);
        }
        let rendered = run_token_committed(&mut engine, token)?;

        let score = engine.token_score();
        let layout = engine.current_layout();
        let margin = score.english.total - score.secondary.total;

        if json {
            writeln!(
                stdout,
                "{{\"keys\":\"{}\",\"layout\":\"{}\",\"rendered\":\"{}\",\"primary_id\":\"{}\",\"secondary_id\":\"{}\",\"primary_total\":{:.4},\"secondary_total\":{:.4},\"margin\":{:+.4}}}",
                escape_json(token),
                escape_json(layout_label(&engine, layout)),
                escape_json(&rendered),
                escape_json(token_label(&engine, Layout::English)),
                escape_json(token_label(&engine, Layout::Secondary)),
                score.english.total,
                score.secondary.total,
                margin,
            )
            .map_err(io_str)?;
        } else {
            writeln!(stdout, "{}\t{}", layout_label(&engine, layout), rendered).map_err(io_str)?;
        }
    }

    Ok(())
}

fn cmd_convert(args: &[String], explicit_config: Option<&Path>) -> Result<(), String> {
    if args.len() != 1 {
        return Err("usage: typeflow convert <KEYS>".into());
    }

    let source = load_config(explicit_config)?;
    let mut engine = build_engine(&source.config)?;
    let mut committed = run_token_committed(&mut engine, &args[0])?;
    let output = engine.force_switch_token();
    apply_action(&output.action, &mut committed);

    println!(
        "{}\t{}",
        layout_label(&engine, engine.current_layout()),
        committed
    );
    Ok(())
}

#[derive(Clone, Debug)]
struct EvalCase {
    name: String,
    keys: String,
    expected: Layout,
}

fn cmd_eval(args: &[String], explicit_config: Option<&Path>) -> Result<(), String> {
    let source = load_config(explicit_config)?;
    let mut engine = build_engine(&source.config)?;
    let cases = eval_cases(args, &engine)?;

    let mut passed = 0usize;
    let mut failed = 0usize;

    for case in &cases {
        engine.reset_layout(Layout::English);
        let rendered = run_token_committed(&mut engine, &case.keys)?;
        let actual = engine.current_layout();

        if actual == case.expected {
            passed += 1;
        } else {
            failed += 1;
            eprintln!(
                "FAIL {}\tkeys={}\texpected={}\tactual={}\trendered={}",
                case.name,
                case.keys,
                layout_label(&engine, case.expected),
                layout_label(&engine, actual),
                rendered,
            );
        }
    }

    println!(
        "eval: {} passed / {} failed / {} total",
        passed,
        failed,
        cases.len()
    );

    if failed == 0 {
        Ok(())
    } else {
        Err("evaluation failed".into())
    }
}

fn eval_cases(args: &[String], engine: &Engine) -> Result<Vec<EvalCase>, String> {
    match args {
        [] => Ok(builtin_eval_cases()),
        [flag] if flag == "--generated" => generated_eval_cases(engine, 5_000),
        [flag, limit] if flag == "--generated" => {
            let limit = limit
                .parse::<usize>()
                .map_err(|e| format!("parse generated eval limit: {e}"))?;
            if limit == 0 {
                return Err("generated eval limit must be > 0".into());
            }
            generated_eval_cases(engine, limit)
        }
        [path] if !path.starts_with("--") => read_eval_cases(Path::new(path)),
        _ => Err("usage: typeflow eval [--generated [limit-per-layout] | <tsv>]".into()),
    }
}

fn read_eval_cases(path: &Path) -> Result<Vec<EvalCase>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let reader = io::BufReader::new(file);
    let mut cases = Vec::new();

    for (idx, line) in reader.lines().enumerate() {
        let line = line.map_err(io_str)?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut parts = trimmed.split('\t');
        let keys = parts
            .next()
            .ok_or_else(|| format!("{}:{} missing keys", path.display(), idx + 1))?;
        let expected = parts
            .next()
            .ok_or_else(|| format!("{}:{} missing expected layout", path.display(), idx + 1))?;

        cases.push(EvalCase {
            name: format!("{}:{}", path.display(), idx + 1),
            keys: keys.to_owned(),
            expected: parse_layout(expected)?,
        });
    }

    Ok(cases)
}

fn builtin_eval_cases() -> Vec<EvalCase> {
    [
        ("ru_privet", "ghbdtn", Layout::Secondary),
        ("ru_privet_caps", "Ghbdtn", Layout::Secondary),
        ("en_typeflow", "typeflow", Layout::English),
        ("en_hello", "hello", Layout::English),
        ("en_http", "http", Layout::English),
        ("en_https_urlish", "https://example.com", Layout::English),
        ("en_kubectl", "kubectl", Layout::English),
        ("en_json", "json", Layout::English),
        ("en_aws", "aws", Layout::English),
        ("en_mixed_digits", "abc123", Layout::English),
        ("en_snake_case", "snake_case", Layout::English),
        ("en_camel_case", "camelCase", Layout::English),
        ("en_acronym", "HTTP", Layout::English),
    ]
    .into_iter()
    .map(|(name, keys, expected)| EvalCase {
        name: name.to_owned(),
        keys: keys.to_owned(),
        expected,
    })
    .collect()
}

fn generated_eval_cases(engine: &Engine, limit_per_layout: usize) -> Result<Vec<EvalCase>, String> {
    let mut cases = builtin_eval_cases();
    cases.extend(generated_dictionary_cases(
        engine,
        Layout::English,
        limit_per_layout,
    )?);
    cases.extend(generated_dictionary_cases(
        engine,
        Layout::Secondary,
        limit_per_layout,
    )?);
    Ok(cases)
}

fn generated_dictionary_cases(
    engine: &Engine,
    expected: Layout,
    limit: usize,
) -> Result<Vec<EvalCase>, String> {
    let pack = engine.bundle().pack(expected);
    let mut words = dictionary_words_by_frequency(pack)?;
    words.retain(|word| word.chars().count() >= engine.config().min_token_len);
    words.truncate(limit);

    let mut cases = Vec::with_capacity(words.len());
    for (idx, word) in words.into_iter().enumerate() {
        let Some(keys) = physical_keys_for_word(engine, &word) else {
            continue;
        };
        cases.push(EvalCase {
            name: format!(
                "generated_{}_{}_{}",
                token_label(engine, expected),
                idx + 1,
                word
            ),
            keys,
            expected,
        });
    }

    Ok(cases)
}

fn dictionary_words_by_frequency(pack: &LanguagePack) -> Result<Vec<String>, String> {
    let mut entries: Vec<(String, u64)> = Vec::new();
    let mut stream = pack.dict.stream();

    while let Some((key, count)) = stream.next() {
        let word = std::str::from_utf8(key)
            .map_err(|e| format!("dictionary {} contains non-UTF-8 key: {e}", pack.id))?;
        entries.push((word.to_owned(), count));
    }

    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    Ok(entries.into_iter().map(|(word, _)| word).collect())
}

fn physical_keys_for_word(engine: &Engine, word: &str) -> Option<String> {
    let bundle = engine.bundle();
    let mut keys = String::new();

    for character in word.chars() {
        let event = bundle.letter_event_from_char(character)?;
        keys.push(bundle.render(event, Layout::English));
    }

    Some(keys)
}

fn parse_layout(value: &str) -> Result<Layout, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "en" | "eng" | "english" => Ok(Layout::English),
        "ru" | "rus" | "russian" | "secondary" => Ok(Layout::Secondary),
        other => Err(format!("unknown expected layout: {other}")),
    }
}

fn cmd_bench(args: &[String], explicit_config: Option<&Path>) -> Result<(), String> {
    if args.len() > 1 {
        return Err("usage: typeflow bench [iterations]".into());
    }
    let iterations = match args.first() {
        Some(value) => value
            .parse::<usize>()
            .map_err(|e| format!("parse iterations: {e}"))?,
        None => 50_000,
    };
    if iterations == 0 {
        return Err("iterations must be > 0".into());
    }

    let source = load_config(explicit_config)?;
    let mut engine = build_engine(&source.config)?;
    let tokens = [
        "ghbdtn",
        "typeflow",
        "http",
        "kubectl",
        "Ghbdtn",
        "snake_case",
    ];
    let started = Instant::now();
    let mut keys = 0usize;

    for idx in 0..iterations {
        let token = tokens[idx % tokens.len()];
        engine.reset_layout(Layout::English);
        run_token_silent(&mut engine, token)?;
        keys += token.chars().count();
    }

    let elapsed = started.elapsed();
    let ns_per_key = elapsed.as_nanos() as f64 / keys as f64;
    println!(
        "bench: {} tokens / {} keys in {:.3}s = {:.1} ns/key",
        iterations,
        keys,
        elapsed.as_secs_f64(),
        ns_per_key
    );
    Ok(())
}

fn cmd_model(args: &[String], explicit_config: Option<&Path>) -> Result<(), String> {
    if !args.is_empty() {
        return Err("usage: typeflow model".into());
    }

    let source = load_config(explicit_config)?;
    let engine = build_engine(&source.config)?;

    for layout in [Layout::English, Layout::Secondary] {
        let pack = engine.bundle().pack(layout);
        println!(
            "{}\tid={}\tscript={}\tlayout={}\tformat={}\tbuild={}\tngrams={}B\tdict={}B\tfingerprint={:016x}",
            pack.display_name,
            pack.id,
            pack.script,
            pack.keyboard_layout,
            pack.metadata.format_version,
            pack.metadata.build_id,
            pack.metadata.ngram_bytes,
            pack.metadata.dict_bytes,
            pack.metadata.fingerprint,
        );
    }

    Ok(())
}

fn cmd_pack(args: &[String], explicit_config: Option<&Path>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("export-ru") => {
            if args.len() != 2 {
                return Err("usage: typeflow pack export-ru <DIR>".into());
            }
            export_embedded_russian_pack(Path::new(&args[1]))
        }
        Some("install") => {
            if args.len() != 2 {
                return Err("usage: typeflow pack install <PACK_DIR>".into());
            }
            install_pack(Path::new(&args[1]), explicit_config)
        }
        Some("list") => {
            if args.len() != 1 {
                return Err("usage: typeflow pack list".into());
            }
            list_packs(explicit_config)
        }
        Some("use") => {
            if args.len() != 2 {
                return Err("usage: typeflow pack use <ID>".into());
            }
            use_pack(args[1].as_str(), explicit_config)
        }
        Some("inspect") => {
            if args.len() != 2 {
                return Err("usage: typeflow pack inspect <PACK_DIR|ID>".into());
            }
            inspect_pack(args[1].as_str(), explicit_config)
        }
        Some(other) => Err(format!(
            "unknown pack subcommand: {other}\n\nusage: typeflow pack <export-ru|install|list|use|inspect>"
        )),
        None => Err("usage: typeflow pack <export-ru|install|list|use|inspect>".into()),
    }
}

fn export_embedded_russian_pack(out_dir: &Path) -> Result<(), String> {
    ensure_empty_or_create_dir(out_dir)?;
    let (ngrams, dict) = LanguageBundle::embedded_secondary_artifacts();
    fs::write(out_dir.join(PACK_NGRAMS_FILE), ngrams)
        .map_err(|e| format!("write {}: {e}", out_dir.join(PACK_NGRAMS_FILE).display()))?;
    fs::write(out_dir.join(PACK_DICT_FILE), dict)
        .map_err(|e| format!("write {}: {e}", out_dir.join(PACK_DICT_FILE).display()))?;

    let manifest = LanguagePackManifest::embedded_russian();
    manifest
        .write_to_dir(out_dir)
        .map_err(|e| format!("write manifest: {e}"))?;

    let pack = LanguagePack::from_pack_dir(out_dir)
        .map_err(|e| format!("validate exported pack {}: {e}", out_dir.display()))?;
    println!(
        "exported {}\t{}\t{}",
        pack.id,
        pack.display_name,
        out_dir.display()
    );
    Ok(())
}

fn install_pack(source_dir: &Path, explicit_config: Option<&Path>) -> Result<(), String> {
    let source_config = load_config(explicit_config)?;
    let install_root = pack_dir_or_error(&source_config.config)?;

    let manifest = LanguagePackManifest::read_from_dir(source_dir)
        .map_err(|e| format!("read pack {}: {e}", source_dir.display()))?;
    let pack = LanguagePack::from_pack_dir(source_dir)
        .map_err(|e| format!("validate pack {}: {e}", source_dir.display()))?;
    let (ngrams_src, dict_src) = manifest
        .artifact_paths(source_dir)
        .map_err(|e| format!("resolve pack artifacts: {e}"))?;

    let target_dir = install_root.join(&pack.id);
    fs::create_dir_all(&target_dir).map_err(|e| format!("create {}: {e}", target_dir.display()))?;
    copy_pack_file(&ngrams_src, &target_dir.join(PACK_NGRAMS_FILE))?;
    copy_pack_file(&dict_src, &target_dir.join(PACK_DICT_FILE))?;
    manifest
        .normalized_for_install()
        .write_to_dir(&target_dir)
        .map_err(|e| format!("write installed manifest: {e}"))?;

    let installed = LanguagePack::from_pack_dir(&target_dir)
        .map_err(|e| format!("validate installed pack {}: {e}", target_dir.display()))?;
    println!(
        "installed {}\t{}\t{}",
        installed.id,
        installed.display_name,
        target_dir.display()
    );
    Ok(())
}

fn list_packs(explicit_config: Option<&Path>) -> Result<(), String> {
    let source = load_config(explicit_config)?;
    let install_root = pack_dir_or_error(&source.config)?;
    println!("pack_dir: {}", install_root.display());
    println!(
        "active_secondary: {}",
        normalized_secondary_id(&source.config)
    );

    if !install_root.is_dir() {
        println!("(none)");
        return Ok(());
    }

    let mut entries = fs::read_dir(&install_root)
        .map_err(|e| format!("read {}: {e}", install_root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_str)?;
    entries.sort_by_key(|entry| entry.file_name());

    let mut listed = 0usize;
    for entry in entries {
        let path = entry.path();
        if !path.join(PACK_MANIFEST_FILE).is_file() {
            continue;
        }
        listed += 1;
        match LanguagePack::from_pack_dir(&path) {
            Ok(pack) => println!(
                "{}\t{}\t{}\t{}\t{:016x}",
                pack.id,
                pack.display_name,
                pack.script,
                pack.keyboard_layout,
                pack.metadata.fingerprint
            ),
            Err(error) => println!(
                "BROKEN\t{}\t{}",
                path.file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_else(|| path.as_os_str().to_string_lossy()),
                error
            ),
        }
    }

    if listed == 0 {
        println!("(none)");
    }
    Ok(())
}

fn use_pack(id: &str, explicit_config: Option<&Path>) -> Result<(), String> {
    validate_cli_pack_id(id)?;
    let config_path = writable_config_path(explicit_config)?;
    let mut config = if config_path.is_file() {
        load_config(Some(&config_path))?.config
    } else {
        Config::default()
    };

    if id != "ru" {
        let install_root = pack_dir_or_error(&config)?;
        let pack_path = install_root.join(id);
        LanguagePack::from_pack_dir(&pack_path).map_err(|e| {
            format!(
                "pack '{id}' is not installed at {}: {e}",
                pack_path.display()
            )
        })?;
    }

    config.language.secondary = id.to_owned();
    config::write_config(&config_path, &config)?;
    println!("configured secondary={id} in {}", config_path.display());
    Ok(())
}

fn inspect_pack(value: &str, explicit_config: Option<&Path>) -> Result<(), String> {
    let source = load_config(explicit_config)?;
    let install_root = pack_dir_or_error(&source.config)?;
    let pack_path = resolve_pack_reference(value, &install_root);

    if value == "ru" && !pack_path.join(PACK_MANIFEST_FILE).is_file() {
        let bundle =
            LanguageBundle::embedded().map_err(|e| format!("load embedded bundle: {e}"))?;
        print_pack_details(&bundle.secondary, "embedded");
        return Ok(());
    }

    let pack = LanguagePack::from_pack_dir(&pack_path)
        .map_err(|e| format!("load pack {}: {e}", pack_path.display()))?;
    print_pack_details(&pack, &pack_path.display().to_string());
    Ok(())
}

fn print_pack_details(pack: &LanguagePack, location: &str) {
    println!("path: {location}");
    println!("id: {}", pack.id);
    println!("display_name: {}", pack.display_name);
    println!("script: {}", pack.script);
    println!("layout: {}", pack.keyboard_layout);
    println!("format_version: {}", pack.metadata.format_version);
    println!("build_id: {}", pack.metadata.build_id);
    println!("source_corpus: {}", pack.metadata.source_corpus);
    println!("source_dictionary: {}", pack.metadata.source_dictionary);
    println!("ngrams: {}B", pack.metadata.ngram_bytes);
    println!("dict: {}B", pack.metadata.dict_bytes);
    println!("fingerprint: {:016x}", pack.metadata.fingerprint);
}

fn pack_dir_or_error(config: &Config) -> Result<PathBuf, String> {
    configured_pack_dir(config).ok_or_else(|| {
        "could not resolve pack directory; set TYPEFLOW_PACK_DIR or [packs].directory".to_owned()
    })
}

fn writable_config_path(explicit_config: Option<&Path>) -> Result<PathBuf, String> {
    explicit_config
        .map(Path::to_path_buf)
        .or_else(config::home_default)
        .ok_or_else(|| {
            "could not locate ~/.config/typeflow/config.toml; pass --config <path>".to_owned()
        })
}

fn resolve_pack_reference(value: &str, install_root: &Path) -> PathBuf {
    let path = PathBuf::from(value);
    if path.exists() || path.join(PACK_MANIFEST_FILE).is_file() || value.contains('/') {
        path
    } else {
        install_root.join(value)
    }
}

fn validate_cli_pack_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("pack id must not be empty".into());
    }
    if !id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(format!(
            "pack id '{id}' contains unsupported characters; use ASCII letters, digits, '-' or '_'"
        ));
    }
    Ok(())
}

fn ensure_empty_or_create_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!("{} exists and is not a directory", path.display()));
        }
        let mut entries =
            fs::read_dir(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if entries.next().is_some() {
            return Err(format!(
                "{} already exists and is not empty",
                path.display()
            ));
        }
        return Ok(());
    }

    fs::create_dir_all(path).map_err(|e| format!("create {}: {e}", path.display()))
}

fn copy_pack_file(source: &Path, dest: &Path) -> Result<(), String> {
    if paths_same(source, dest) {
        return Ok(());
    }
    fs::copy(source, dest)
        .map(|_| ())
        .map_err(|e| format!("copy {} -> {}: {e}", source.display(), dest.display()))
}

fn paths_same(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn escape_json(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if (c as u32) < 0x20 => output.push_str(&format!("\\u{:04x}", c as u32)),
            c => output.push(c),
        }
    }
    output
}

fn cmd_repl(explicit_config: Option<&Path>) -> Result<(), String> {
    let source = load_config(explicit_config)?;
    let mut engine = build_engine(&source.config)?;
    let config_label = source
        .path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<defaults>".to_owned());
    let mut history: Vec<String> = Vec::new();
    let mut committed: String = String::new();

    let mut stdout = io::stdout();
    terminal::enable_raw_mode().map_err(io_str)?;
    let _guard = RawModeGuard;
    stdout
        .execute(terminal::Clear(ClearType::All))
        .map_err(io_str)?;

    loop {
        redraw(&mut engine, &history, &committed, &config_label).map_err(io_str)?;
        let evt = event::read().map_err(io_str)?;
        let Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            ..
        }) = evt
        else {
            continue;
        };
        if kind != KeyEventKind::Press {
            continue;
        }
        match (code, modifiers) {
            (KeyCode::Esc, _) => break,
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
            (KeyCode::Backspace, _) => {
                let output = engine.process(InputEvent::Backspace);
                committed.pop();
                history.push(format!(
                    "Backspace -> {}",
                    action_label(&engine, &output.action)
                ));
            }
            (KeyCode::Char(' '), _) => {
                engine.process(InputEvent::EndToken);
                committed.push(' ');
                history.push("EndToken (space)".into());
            }
            (KeyCode::Enter, _) => {
                engine.process(InputEvent::EndToken);
                committed.push('\n');
                history.push("EndToken (enter)".into());
            }
            (KeyCode::Char(character), _) => {
                let output = engine.process(input_event_for_char(&engine, character));
                apply_action(&output.action, &mut committed);
                history.push(format!(
                    "'{character}' -> decision={} action={}",
                    decision_label(&engine, output.decision),
                    action_label(&engine, &output.action)
                ));
            }
            _ => {}
        }
    }

    Ok(())
}

/// Mirrors what an IMK host would do with the engine's action: commit a char,
/// or replace the trailing N chars with the engine's preferred rendering.
fn apply_action(action: &Action, committed: &mut String) {
    match action {
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
        Action::Keep | Action::ResetToken => {}
    }
}

fn cmd_config(args: &[String], explicit_config: Option<&Path>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("init") => {
            let target = args.get(1).map(PathBuf::from).or_else(config::home_default);
            let target = target.ok_or_else(|| {
                "could not locate ~/.config/typeflow/config.toml; pass an explicit path".to_owned()
            })?;
            if target.exists() {
                return Err(format!(
                    "{} already exists; remove it or pass an alternative path",
                    target.display()
                ));
            }
            config::write_default_template(&target)?;
            println!("wrote default config to {}", target.display());
            Ok(())
        }
        Some("show") => {
            let source = load_config(explicit_config)?;
            match &source.path {
                Some(path) => println!("# loaded from {}\n", path.display()),
                None => println!("# no config file found; using built-in defaults\n"),
            }
            let serialized = toml::to_string_pretty(&source.config)
                .map_err(|e| format!("serialize config: {e}"))?;
            print!("{serialized}");
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown config subcommand: {other}\n\nusage: typeflow config <init|show> [path]"
        )),
        None => Err("usage: typeflow config <init|show> [path]".into()),
    }
}

struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().execute(terminal::Clear(ClearType::All));
        let _ = io::stdout().execute(cursor::MoveTo(0, 0));
    }
}

fn redraw(
    engine: &mut Engine,
    history: &[String],
    committed: &str,
    config_label: &str,
) -> io::Result<()> {
    let mut stdout = io::stdout();
    stdout.execute(terminal::Clear(ClearType::All))?;
    stdout.execute(cursor::MoveTo(0, 0))?;

    let candidates = engine.token_candidates();
    let score = engine.token_score();

    let mut buf = String::new();
    let _ = writeln_cr(
        &mut buf,
        "TypeFlow REPL — Esc/Ctrl-C quit | Backspace delete | Space/Enter end-of-token",
    );
    let _ = writeln_cr(&mut buf, format!("config: {config_label}"));
    let _ = writeln_cr(&mut buf, "");
    let _ = writeln_cr(&mut buf, "What you would see in TextEdit:");
    let _ = writeln_cr(&mut buf, format!("  {}", visible_committed(committed)));
    let _ = writeln_cr(&mut buf, "");
    let _ = writeln_cr(
        &mut buf,
        format!(
            "Current layout: {}",
            layout_label(engine, engine.current_layout())
        ),
    );
    let _ = writeln_cr(
        &mut buf,
        format!(
            "Token ({}): {}",
            token_label(engine, Layout::English),
            candidates.english
        ),
    );
    let _ = writeln_cr(
        &mut buf,
        format!(
            "Token ({}): {}",
            token_label(engine, Layout::Secondary),
            candidates.secondary
        ),
    );
    let _ = writeln_cr(&mut buf, "");
    let _ = writeln_cr(
        &mut buf,
        format_score_line(&score.english, &score_label(engine, Layout::English)),
    );
    let _ = writeln_cr(
        &mut buf,
        format_score_line(&score.secondary, &score_label(engine, Layout::Secondary)),
    );
    let _ = writeln_cr(&mut buf, "");

    let verdict = format_margin_verdict(engine, &score, engine.config());
    let _ = writeln_cr(
        &mut buf,
        format!(
            "Margin ({} - {}): {verdict}",
            score_label(engine, Layout::English),
            score_label(engine, Layout::Secondary)
        ),
    );
    let _ = writeln_cr(&mut buf, "");
    let _ = writeln_cr(&mut buf, "Recent events (newest first):");
    for line in history.iter().rev().take(8) {
        let _ = writeln_cr(&mut buf, format!("  {line}"));
    }

    stdout.write_all(buf.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

fn writeln_cr(buf: &mut String, line: impl AsRef<str>) -> std::fmt::Result {
    buf.write_str(line.as_ref())?;
    buf.write_str("\r\n")
}

/// Renders committed text in a single-line, escape-safe form for the REPL panel.
/// Newlines become `⏎`, leading-edge tail-only display so very long sessions stay readable.
fn visible_committed(text: &str) -> String {
    const TAIL_CHARS: usize = 100;
    let chars: Vec<char> = text.chars().collect();
    let start = chars.len().saturating_sub(TAIL_CHARS);
    let prefix = if start > 0 { "…" } else { "" };
    let mut out = String::with_capacity(prefix.len() + chars.len() - start + 4);
    out.push_str(prefix);
    for character in &chars[start..] {
        match character {
            '\n' => out.push('⏎'),
            '\r' => {}
            c => out.push(*c),
        }
    }
    if out.is_empty() {
        out.push_str("(empty)");
    }
    out
}

fn format_score_line(score: &LayoutScore, label: &str) -> String {
    format!(
        "{label}  total={:>8.2}  bigram={:>8.2}  trigram={:>8.2}  exact={:>5.2}(n={})  prefix={:>5.2}(sum={})",
        score.total,
        score.bigram,
        score.trigram,
        score.dict_exact_bonus,
        score.exact_count,
        score.dict_prefix_bonus,
        score.prefix_sum,
    )
}

fn print_score_lines(engine: &Engine, score: &ScoreAnalysis) {
    println!(
        "{}",
        format_score_line(&score.english, &score_label(engine, Layout::English))
    );
    println!(
        "{}",
        format_score_line(&score.secondary, &score_label(engine, Layout::Secondary))
    );
}

fn layout_label(engine: &Engine, layout: Layout) -> &str {
    engine.bundle().display_name(layout)
}

fn token_label(engine: &Engine, layout: Layout) -> &str {
    engine.bundle().pack(layout).id.as_str()
}

fn score_label(engine: &Engine, layout: Layout) -> String {
    token_label(engine, layout).to_ascii_uppercase()
}

fn print_margin_line(engine: &Engine, score: &ScoreAnalysis, config: &EngineConfig) {
    println!(
        "Margin ({} - {}): {}",
        score_label(engine, Layout::English),
        score_label(engine, Layout::Secondary),
        format_margin_verdict(engine, score, config)
    );
}

fn format_margin_verdict(engine: &Engine, score: &ScoreAnalysis, config: &EngineConfig) -> String {
    let margin = score.english.total - score.secondary.total;
    let (threshold, threshold_label) = if margin >= 0.0 {
        margin_threshold(score.english, config)
    } else {
        margin_threshold(score.secondary, config)
    };

    if margin.abs() < threshold {
        format!(
            "{:+.2}  threshold={:.2} ({})  -> keep current",
            margin, threshold, threshold_label
        )
    } else if margin > 0.0 {
        format!(
            "{:+.2}  threshold={:.2} ({})  -> Use({})",
            margin,
            threshold,
            threshold_label,
            layout_label(engine, Layout::English)
        )
    } else {
        format!(
            "{:+.2}  threshold={:.2} ({})  -> Use({})",
            margin,
            threshold,
            threshold_label,
            layout_label(engine, Layout::Secondary)
        )
    }
}

fn decision_label(engine: &Engine, decision: Decision) -> String {
    match decision {
        Decision::Keep => "Keep".to_owned(),
        Decision::Bypass => "Bypass".to_owned(),
        Decision::Use(layout) => format!("Use({})", layout_label(engine, layout)),
    }
}

fn action_label(engine: &Engine, action: &Action) -> String {
    match action {
        Action::Keep => "Keep".to_owned(),
        Action::Commit(character) => format!("Commit({character:?})"),
        Action::ReplaceToken {
            old_len,
            replacement,
            layout,
        } => format!(
            "ReplaceToken {{ old_len: {old_len}, replacement: {replacement:?}, layout: {} }}",
            layout_label(engine, *layout)
        ),
        Action::ResetToken => "ResetToken".to_owned(),
    }
}

fn margin_threshold(score: LayoutScore, config: &EngineConfig) -> (f32, &'static str) {
    if has_dictionary_evidence(score) {
        (config.confidence_margin, "dictionary")
    } else {
        (config.ngram_only_confidence_margin, "ngram-only")
    }
}

fn io_str(error: io::Error) -> String {
    format!("{error}")
}
