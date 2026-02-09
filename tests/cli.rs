use std::{
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn expected_output() -> String {
    fs::read_to_string(fixture_path("mixed_outputs.kanshi")).unwrap()
}

fn binary_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kanshi-generate"))
}

fn setup_fake_wlr_randr() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let script_path = temp_dir.path().join("wlr-randr");
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

case "${WLR_RANDR_BEHAVIOR:-ok}" in
  ok)
    cat "${WLR_RANDR_FIXTURE}"
    ;;
  fail)
    echo "failed to connect to display" >&2
    exit 1
    ;;
  invalid)
    printf '{invalid json'
    ;;
  empty)
    exit 0
    ;;
  *)
    echo "unknown test behavior" >&2
    exit 2
    ;;
esac
"#;
    fs::write(&script_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let permissions = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    temp_dir
}

fn run_with_fake_wlr_randr(args: &[&str], behavior: &str) -> Output {
    let fixture = fixture_path("mixed_outputs.json");
    let fake_bin_dir = setup_fake_wlr_randr();
    let original_path = env::var("PATH").unwrap_or_default();

    let mut command = binary_command();
    command.args(args);
    command.env("WLR_RANDR_BEHAVIOR", behavior);
    command.env("WLR_RANDR_FIXTURE", fixture);
    command.env(
        "PATH",
        format!("{}:{original_path}", fake_bin_dir.path().display()),
    );
    command.output().unwrap()
}

#[test]
fn cli_generates_profile_from_wlr_randr_command() {
    let output = run_with_fake_wlr_randr(&["docked"], "ok");
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected_output());
}

#[test]
fn cli_surfaces_wlr_randr_stderr_on_failure() {
    let output = run_with_fake_wlr_randr(&["docked"], "fail");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to collect data from wlr-randr"));
    assert!(stderr.contains("failed to connect to display"));
}

#[test]
fn cli_errors_for_invalid_json() {
    let output = run_with_fake_wlr_randr(&["docked"], "invalid");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to parse wlr-randr output JSON"));
}

#[test]
fn cli_errors_for_empty_stdout() {
    let output = run_with_fake_wlr_randr(&["docked"], "empty");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("returned empty stdout"));
}

#[test]
fn cli_reads_input_file_and_writes_output_file() {
    let fixture = fixture_path("mixed_outputs.json");
    let output_dir = TempDir::new().unwrap();
    let output_path = output_dir.path().join("generated.kanshi");

    let mut command = binary_command();
    let output = command
        .args([
            "docked",
            "--input-json",
            fixture.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let generated = fs::read_to_string(output_path).unwrap();
    assert_eq!(generated, expected_output());
}

#[test]
fn cli_reads_json_from_stdin() {
    let fixture = fs::read(fixture_path("mixed_outputs.json")).unwrap();
    let mut command = binary_command();
    let mut child = command
        .args(["docked", "--input-json", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(&fixture).unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected_output());
}
