#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn typeclaw() -> Command {
    Command::new(env!("CARGO_BIN_EXE_typeclaw"))
}

fn empty_config(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "typeclaw-cli-{test_name}-{}-{unique}.toml",
        std::process::id()
    ));
    fs::write(&path, "").expect("write empty test config");
    path
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn version_flag_exits_successfully() {
    let output = typeclaw()
        .arg("--version")
        .output()
        .expect("run typeclaw --version");

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("typeclaw ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn predict_uses_embedded_ukrainian_bundle() {
    let config = empty_config("predict");
    let output = typeclaw()
        .args(["--config", config.to_str().expect("utf-8 temp path")])
        .args(["predict", "ghsdbn"])
        .output()
        .expect("run typeclaw predict");

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Ukrainian\tпривіт\n"
    );
}

#[test]
fn predict_handles_secondary_letters_on_english_punctuation_keys() {
    let config = empty_config("predict-separator-keys");
    let output = typeclaw()
        .args(["--config", config.to_str().expect("utf-8 temp path")])
        .args(["predict", ",f,f"])
        .output()
        .expect("run typeclaw predict");

    assert_success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Ukrainian\tбаба\n");
}

#[test]
fn predict_json_outputs_one_record_per_token() {
    let config = empty_config("predict-json");
    let output = typeclaw()
        .args(["--config", config.to_str().expect("utf-8 temp path")])
        .args(["predict", "--json", "ghsdbn"])
        .output()
        .expect("run typeclaw predict --json");

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let record: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("predict --json emits valid json");
    assert_eq!(record["keys"], "ghsdbn");
    assert_eq!(record["layout"], "Ukrainian");
    assert_eq!(record["rendered"], "привіт");
    assert!(
        record["secondary_total"]
            .as_f64()
            .expect("secondary_total number")
            > record["primary_total"]
                .as_f64()
                .expect("primary_total number")
    );
    assert!(record["margin"].as_f64().expect("margin number") < -1.0);
}

#[test]
fn usage_errors_exit_2() {
    let output = typeclaw()
        .arg("--bogus")
        .output()
        .expect("run typeclaw --bogus");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument '--bogus'"));
}
