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

fn run_with_input_json(args: &[&str], configure: impl FnOnce(&mut Command)) -> Output {
    let fixture = fixture_path("mixed_outputs.json");

    let mut command = binary_command();
    command.args(args);
    command.arg("--input-json");
    command.arg(fixture);
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

    let output = run_with_input_json(&["docked"], |command| {
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

    let output = run_with_input_json(
        &["docked", "--config", config_path.to_str().unwrap()],
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

    let output = run_with_input_json(
        &["docked", "--config", config_path.to_str().unwrap()],
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

    let output = run_with_input_json(
        &["docked", "--config", config_path.to_str().unwrap()],
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

    let output = run_with_input_json(&["docked", "--stdout"], |command| {
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

    let output = run_with_input_json(
        &["docked", "--output", out_path.to_str().unwrap()],
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
    let output = run_with_input_json(&["docked", "--stdout", "--output", "x"], |_| {});
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn cli_rejects_config_with_stdout() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("config");
    let output = run_with_input_json(
        &[
            "docked",
            "--stdout",
            "--config",
            config_path.to_str().unwrap(),
        ],
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
    let output = run_with_input_json(
        &[
            "docked",
            "--output",
            "x",
            "--config",
            config_path.to_str().unwrap(),
        ],
        |_| {},
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn cli_errors_for_invalid_json() {
    let temp = TempDir::new().unwrap();
    let invalid_path = temp.path().join("invalid.json");
    fs::write(&invalid_path, "{invalid json").unwrap();

    let mut command = binary_command();
    let output = command
        .args([
            "docked",
            "--stdout",
            "--input-json",
            invalid_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to parse input output JSON"));
}

#[test]
fn cli_errors_for_empty_json() {
    let temp = TempDir::new().unwrap();
    let empty_path = temp.path().join("empty.json");
    fs::write(&empty_path, "").unwrap();

    let mut command = binary_command();
    let output = command
        .args([
            "docked",
            "--stdout",
            "--input-json",
            empty_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to parse input output JSON"));
}

#[test]
fn cli_live_mode_reports_wayland_connect_error() {
    let runtime = TempDir::new().unwrap();

    let mut command = binary_command();
    let output = command
        .args(["docked", "--stdout"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", "wayland-not-existing")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to collect output state from Wayland protocol"));
    assert!(stderr.contains("failed to connect to Wayland compositor"));
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

    let output = run_with_input_json(&["docked", "--config", link_path.to_str().unwrap()], |_| {});

    assert!(output.status.success());
    assert!(
        fs::symlink_metadata(&link_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&target_path).unwrap(), expected_output());
}
