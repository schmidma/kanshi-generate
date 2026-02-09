use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
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

fn run_with_fake_wlr_randr(
    args: &[&str],
    behavior: &str,
    configure: impl FnOnce(&mut Command),
) -> Output {
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
    configure(&mut command);
    command.output().unwrap()
}

#[test]
fn cli_default_mode_updates_existing_profile_in_default_config_path() {
    let xdg_config_home = TempDir::new().unwrap();
    let config_path = xdg_config_home.path().join("kanshi").join("config");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(
        &config_path,
        "profile docked {\n  output \"old\" disable\n}\n\nprofile untouched {\n  output \"x\" disable\n}\n",
    )
    .unwrap();

    let output = run_with_fake_wlr_randr(&["docked"], "ok", |command| {
        command.env("XDG_CONFIG_HOME", xdg_config_home.path());
    });

    assert!(output.status.success());
    assert!(output.stdout.is_empty());

    let updated = fs::read_to_string(config_path).unwrap();
    assert!(updated.contains(&expected_output()));
    assert!(updated.contains("profile untouched"));
    assert!(!updated.contains("output \"old\" disable"));
}

#[test]
fn cli_config_flag_updates_given_file_path() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("custom.conf");
    fs::write(
        &config_path,
        "profile docked {\n  output \"old\" disable\n}\n",
    )
    .unwrap();

    let output = run_with_fake_wlr_randr(
        &["docked", "--config", config_path.to_str().unwrap()],
        "ok",
        |_| {},
    );

    assert!(output.status.success());
    let updated = fs::read_to_string(config_path).unwrap();
    assert_eq!(updated, expected_output());
}

#[test]
fn cli_appends_profile_if_missing() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("config");
    fs::write(&config_path, "profile alpha {\n  output \"x\" disable\n}\n").unwrap();

    let output = run_with_fake_wlr_randr(
        &["docked", "--config", config_path.to_str().unwrap()],
        "ok",
        |_| {},
    );

    assert!(output.status.success());
    let updated = fs::read_to_string(config_path).unwrap();
    assert!(updated.contains("profile alpha"));
    assert!(updated.ends_with(&expected_output()));
}

#[test]
fn cli_fails_and_keeps_file_unchanged_for_duplicate_profiles() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("config");
    let initial = "profile docked {\n}\n\nprofile docked {\n}\n";
    fs::write(&config_path, initial).unwrap();

    let output = run_with_fake_wlr_randr(
        &["docked", "--config", config_path.to_str().unwrap()],
        "ok",
        |_| {},
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate profile `docked`"));
    assert_eq!(fs::read_to_string(config_path).unwrap(), initial);
}

#[test]
fn cli_stdout_mode_prints_profile_and_does_not_edit_config() {
    let xdg_config_home = TempDir::new().unwrap();
    let config_path = xdg_config_home.path().join("kanshi").join("config");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let initial = "profile docked {\n  output \"old\" disable\n}\n";
    fs::write(&config_path, initial).unwrap();

    let output = run_with_fake_wlr_randr(&["docked", "--stdout"], "ok", |command| {
        command.env("XDG_CONFIG_HOME", xdg_config_home.path());
    });

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected_output());
    assert_eq!(fs::read_to_string(config_path).unwrap(), initial);
}

#[test]
fn cli_output_mode_writes_raw_profile_without_parsing_config() {
    let temp = TempDir::new().unwrap();
    let out_path = temp.path().join("generated.kanshi");

    let xdg_config_home = TempDir::new().unwrap();
    let broken_config = xdg_config_home.path().join("kanshi").join("config");
    fs::create_dir_all(broken_config.parent().unwrap()).unwrap();
    fs::write(&broken_config, "profile broken {\n  output \"x\" disable\n").unwrap();

    let output = run_with_fake_wlr_randr(
        &["docked", "--output", out_path.to_str().unwrap()],
        "ok",
        |command| {
            command.env("XDG_CONFIG_HOME", xdg_config_home.path());
        },
    );

    assert!(output.status.success());
    assert_eq!(fs::read_to_string(out_path).unwrap(), expected_output());
    assert_eq!(
        fs::read_to_string(broken_config).unwrap(),
        "profile broken {\n  output \"x\" disable\n"
    );
}

#[test]
fn cli_rejects_stdout_and_output_together() {
    let output = run_with_fake_wlr_randr(&["docked", "--stdout", "--output", "x"], "ok", |_| {});
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn cli_rejects_config_with_stdout() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("config");
    let output = run_with_fake_wlr_randr(
        &[
            "docked",
            "--stdout",
            "--config",
            config_path.to_str().unwrap(),
        ],
        "ok",
        |_| {},
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn cli_rejects_config_with_output() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("config");
    let output = run_with_fake_wlr_randr(
        &[
            "docked",
            "--output",
            "x",
            "--config",
            config_path.to_str().unwrap(),
        ],
        "ok",
        |_| {},
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn cli_surfaces_wlr_randr_stderr_on_failure() {
    let output = run_with_fake_wlr_randr(&["docked", "--stdout"], "fail", |_| {});
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to collect data from wlr-randr"));
    assert!(stderr.contains("failed to connect to display"));
}

#[test]
fn cli_errors_for_invalid_json() {
    let output = run_with_fake_wlr_randr(&["docked", "--stdout"], "invalid", |_| {});
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to parse wlr-randr output JSON"));
}

#[test]
fn cli_errors_for_empty_stdout() {
    let output = run_with_fake_wlr_randr(&["docked", "--stdout"], "empty", |_| {});
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("returned empty stdout"));
}

#[cfg(unix)]
#[test]
fn cli_updates_symlink_target_without_replacing_symlink() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let target_path = temp.path().join("real-config");
    fs::write(
        &target_path,
        "profile docked {\n  output \"old\" disable\n}\n",
    )
    .unwrap();

    let link_path = temp.path().join("config-link");
    symlink(&target_path, &link_path).unwrap();

    let output = run_with_fake_wlr_randr(
        &["docked", "--config", link_path.to_str().unwrap()],
        "ok",
        |_| {},
    );

    assert!(output.status.success());
    assert!(
        fs::symlink_metadata(&link_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&target_path).unwrap(), expected_output());
}
