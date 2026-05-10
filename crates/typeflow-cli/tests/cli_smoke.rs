#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn typeflow() -> Command {
    Command::new(env!("CARGO_BIN_EXE_typeflow"))
}

fn empty_config(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "typeflow-cli-{test_name}-{}-{unique}.toml",
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
    let output = typeflow()
        .arg("--version")
        .output()
        .expect("run typeflow --version");

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("typeflow ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn predict_uses_embedded_ukrainian_bundle() {
    let config = empty_config("predict");
    let output = typeflow()
        .args(["--config", config.to_str().expect("utf-8 temp path")])
        .args(["predict", "ghsdbn"])
        .output()
        .expect("run typeflow predict");

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Ukrainian\tпривіт\n"
    );
}

#[test]
fn predict_json_outputs_one_record_per_token() {
    let config = empty_config("predict-json");
    let output = typeflow()
        .args(["--config", config.to_str().expect("utf-8 temp path")])
        .args(["predict", "--json", "ghsdbn"])
        .output()
        .expect("run typeflow predict --json");

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"keys\":\"ghsdbn\""));
    assert!(stdout.contains("\"layout\":\"Ukrainian\""));
    assert!(stdout.contains("\"rendered\":\"привіт\""));
}

#[test]
fn usage_errors_exit_2() {
    let output = typeflow()
        .arg("--bogus")
        .output()
        .expect("run typeflow --bogus");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument '--bogus'"));
}
