use std::{fmt::Write as _, process::Command};

use serde::Deserialize;
use thiserror::Error;

const WLR_RANDR_BINARY: &str = "wlr-randr";
const WLR_RANDR_ARGS: [&str; 1] = ["--json"];

#[derive(Debug, Error)]
pub enum GenerateError {
    #[error("failed to execute `{command}`")]
    SpawnCommand {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{command}` exited unsuccessfully ({status}): {stderr}")]
    CommandFailed {
        command: String,
        status: String,
        stderr: String,
    },
    #[error("`{command}` returned empty stdout")]
    EmptyCommandOutput { command: String },
    #[error("failed to parse wlr-randr output JSON")]
    ParseJson(#[source] serde_json::Error),
    #[error("profile name cannot be empty")]
    EmptyProfileName,
    #[error("output `{output}` is enabled but has no current or preferred mode")]
    MissingMode { output: String },
    #[error("output `{output}` is enabled but has no position")]
    MissingPosition { output: String },
    #[error("output `{output}` is enabled but has no scale")]
    MissingScale { output: String },
    #[error("failed to format kanshi profile")]
    Format,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct WlrStatus(Vec<Output>);

#[derive(Debug, Deserialize)]
struct Output {
    name: String,
    #[serde(default)]
    make: String,
    #[serde(default)]
    model: String,
    serial: Option<String>,
    enabled: bool,
    #[serde(default)]
    modes: Vec<Mode>,
    position: Option<Position>,
    scale: Option<f64>,
}

impl Output {
    fn identifier(&self) -> String {
        let mut segments = Vec::with_capacity(3);
        if !self.make.trim().is_empty() {
            segments.push(self.make.as_str());
        }
        if !self.model.trim().is_empty() {
            segments.push(self.model.as_str());
        }
        if let Some(serial) = self.serial.as_deref()
            && !serial.trim().is_empty()
        {
            segments.push(serial);
        }

        if segments.is_empty() {
            self.name.clone()
        } else {
            segments.join(" ")
        }
    }

    fn active_mode(&self) -> Option<&Mode> {
        self.modes
            .iter()
            .find(|mode| mode.current)
            .or_else(|| self.modes.iter().find(|mode| mode.preferred))
    }
}

#[derive(Debug, Deserialize)]
struct Mode {
    width: u32,
    height: u32,
    refresh: f64,
    preferred: bool,
    current: bool,
}

#[derive(Debug, Deserialize)]
struct Position {
    x: i32,
    y: i32,
}

pub fn capture_wlr_randr_json() -> Result<Vec<u8>, GenerateError> {
    let command = format!("{WLR_RANDR_BINARY} {}", WLR_RANDR_ARGS.join(" "));
    let output = Command::new(WLR_RANDR_BINARY)
        .args(WLR_RANDR_ARGS)
        .output()
        .map_err(|source| GenerateError::SpawnCommand {
            command: command.clone(),
            source,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(GenerateError::CommandFailed {
            command,
            status: output.status.to_string(),
            stderr: if stderr.is_empty() {
                String::from("<no stderr>")
            } else {
                stderr
            },
        });
    }

    if output.stdout.is_empty() {
        return Err(GenerateError::EmptyCommandOutput { command });
    }

    Ok(output.stdout)
}

pub fn generate_profile_from_slice(
    profile_name: &str,
    wlr_randr_json: &[u8],
) -> Result<String, GenerateError> {
    if profile_name.trim().is_empty() {
        return Err(GenerateError::EmptyProfileName);
    }

    let status: WlrStatus =
        serde_json::from_slice(wlr_randr_json).map_err(GenerateError::ParseJson)?;
    render_profile(profile_name, &status.0)
}

fn render_profile(profile_name: &str, outputs: &[Output]) -> Result<String, GenerateError> {
    let mut profile = String::with_capacity(32 + outputs.len() * 128);
    writeln!(&mut profile, "profile {profile_name} {{").map_err(|_| GenerateError::Format)?;

    for output in outputs {
        let output_id = escape_kanshi_quoted(&output.identifier());
        if output.enabled {
            let mode = output
                .active_mode()
                .ok_or_else(|| GenerateError::MissingMode {
                    output: output.name.clone(),
                })?;
            let position =
                output
                    .position
                    .as_ref()
                    .ok_or_else(|| GenerateError::MissingPosition {
                        output: output.name.clone(),
                    })?;
            let scale = output.scale.ok_or_else(|| GenerateError::MissingScale {
                output: output.name.clone(),
            })?;
            writeln!(
                &mut profile,
                "  output \"{output_id}\" mode {}x{}@{:.2}Hz position {},{} scale {:.2}",
                mode.width, mode.height, mode.refresh, position.x, position.y, scale
            )
            .map_err(|_| GenerateError::Format)?;
        } else {
            writeln!(&mut profile, "  output \"{output_id}\" disable")
                .map_err(|_| GenerateError::Format)?;
        }
    }

    profile.push_str("}\n");
    Ok(profile)
}

fn escape_kanshi_quoted(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{GenerateError, generate_profile_from_slice};

    #[test]
    fn renders_fixture_with_enabled_and_disabled_outputs() {
        let json = include_str!("../tests/fixtures/mixed_outputs.json");
        let expected = include_str!("../tests/fixtures/mixed_outputs.kanshi");
        let rendered = generate_profile_from_slice("docked", json.as_bytes()).unwrap();
        assert_eq!(rendered, expected);
    }

    #[test]
    fn picks_current_mode_first() {
        let json = r#"[
          {
            "name":"DP-1",
            "make":"Dell",
            "model":"U2723",
            "serial":"ABC123",
            "enabled":true,
            "modes":[
              {"width":1920,"height":1080,"refresh":60.0,"preferred":true,"current":false},
              {"width":2560,"height":1440,"refresh":59.95,"preferred":false,"current":true}
            ],
            "position":{"x":10,"y":20},
            "scale":1.0
          }
        ]"#;
        let rendered = generate_profile_from_slice("desk", json.as_bytes()).unwrap();
        assert!(rendered.contains("mode 2560x1440@59.95Hz"));
    }

    #[test]
    fn falls_back_to_preferred_mode() {
        let json = r#"[
          {
            "name":"DP-1",
            "make":"Dell",
            "model":"U2723",
            "serial":"ABC123",
            "enabled":true,
            "modes":[
              {"width":1920,"height":1080,"refresh":60.0,"preferred":true,"current":false}
            ],
            "position":{"x":0,"y":0},
            "scale":1.0
          }
        ]"#;
        let rendered = generate_profile_from_slice("desk", json.as_bytes()).unwrap();
        assert!(rendered.contains("mode 1920x1080@60.00Hz"));
    }

    #[test]
    fn errors_when_enabled_output_has_no_current_or_preferred_mode() {
        let json = r#"[
          {
            "name":"DP-1",
            "make":"Dell",
            "model":"U2723",
            "serial":"ABC123",
            "enabled":true,
            "modes":[
              {"width":1920,"height":1080,"refresh":60.0,"preferred":false,"current":false}
            ],
            "position":{"x":0,"y":0},
            "scale":1.0
          }
        ]"#;
        let err = generate_profile_from_slice("desk", json.as_bytes()).unwrap_err();
        assert!(matches!(err, GenerateError::MissingMode { .. }));
    }

    #[test]
    fn keeps_negative_coordinates() {
        let json = r#"[
          {
            "name":"DP-2",
            "make":"Dell",
            "model":"P2723D",
            "serial":"2ZZ6714",
            "enabled":true,
            "modes":[
              {"width":2560,"height":1440,"refresh":59.951,"preferred":true,"current":true}
            ],
            "position":{"x":-2560,"y":300},
            "scale":1.25
          }
        ]"#;
        let rendered = generate_profile_from_slice("desk", json.as_bytes()).unwrap();
        assert!(rendered.contains("position -2560,300"));
    }

    #[test]
    fn omits_unknown_serial_placeholder() {
        let json = r#"[
          {
            "name":"eDP-1",
            "make":"AU Optronics",
            "model":"0xD291",
            "serial":null,
            "enabled":false,
            "modes":[
              {"width":1920,"height":1200,"refresh":60.0,"preferred":true,"current":false}
            ]
          }
        ]"#;
        let rendered = generate_profile_from_slice("mobile", json.as_bytes()).unwrap();
        assert!(rendered.contains("output \"AU Optronics 0xD291\" disable"));
        assert!(!rendered.contains("Unknown"));
    }
}
