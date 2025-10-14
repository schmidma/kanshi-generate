#![warn(clippy::pedantic)]
use std::{
    fmt::{self, Write as _},
    process::Command,
};

use clap::Parser;
use color_eyre::{
    Result,
    eyre::{Context, ContextCompat as _},
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct WlrStatus(Vec<Output>);

impl WlrStatus {
    fn to_kanshi(&self, name: &str) -> Result<String> {
        let mut result = String::new();
        writeln!(&mut result, "profile {name} {{")?;
        for output in &self.0 {
            output
                .to_kanshi(&mut result)
                .wrap_err_with(|| format!("failed to convert output {} to kanshi", output.name))?;
        }
        result.push_str("}\n");
        Ok(result)
    }
}

#[derive(Debug, Deserialize)]
struct Output {
    name: String,
    make: String,
    model: String,
    serial: Option<String>,
    enabled: bool,
    modes: Vec<Mode>,
    position: Position,
    scale: f32,
}

impl Output {
    fn to_kanshi(&self, mut writer: impl fmt::Write) -> Result<()> {
        if self.enabled {
            let active_mode = self
                .modes
                .iter()
                .find(|mode| mode.current)
                .or_else(|| self.modes.iter().find(|mode| mode.preferred))
                .wrap_err_with(|| {
                    format!("no active or preferred mode found for output {}", self.name)
                })?;
            writeln!(
                writer,
                "  output \"{make} {model} {serial}\" mode {width}x{height}@{refresh:.2}Hz position {x},{y} scale {scale:.2}",
                make = self.make,
                model = self.model,
                serial = self.serial.as_deref().unwrap_or("Unknown"),
                width = active_mode.width,
                height = active_mode.height,
                refresh = active_mode.refresh,
                x = self.position.x,
                y = self.position.y,
                scale = self.scale
            )?;
        } else {
            writeln!(writer, "output {} disable", self.name)?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct Mode {
    width: u32,
    height: u32,
    refresh: f32,
    preferred: bool,
    current: bool,
}

#[derive(Debug, Deserialize)]
struct Position {
    x: u32,
    y: u32,
}

#[derive(Debug, Parser)]
struct Arguments {
    /// Profile name
    name: String,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Arguments::parse();

    let mut command = Command::new("wlr-randr");
    command.arg("--json");
    let output = command.output().wrap_err("failed to execute wlr-randr")?;
    let status: WlrStatus =
        serde_json::from_slice(&output.stdout).wrap_err("failed to parse wlr-randr output")?;

    let kanshi = status
        .to_kanshi(&args.name)
        .wrap_err("failed to convert to kanshi")?;
    println!("{kanshi}");
    Ok(())
}
