use std::{
    fs,
    io::{self, Read as _},
    path::PathBuf,
};

use clap::Parser;
use color_eyre::{Result, eyre::Context as _};
use kanshi_generate::{
    collect_outputs_wayland, generate_profile_from_outputs, generate_profile_from_slice,
    resolve_default_kanshi_config_path, upsert_profile_in_file,
};

#[derive(Debug, Parser)]
#[command(
    about = "Generate a kanshi profile from Wayland output-management state",
    version,
    author
)]
struct Arguments {
    /// Profile name
    name: String,
    /// Read JSON from a file path or '-' for stdin instead of querying Wayland output-management protocol
    #[arg(long, value_name = "PATH")]
    input_json: Option<String>,
    /// Override kanshi config file path (default: $XDG_CONFIG_HOME/kanshi/config or $HOME/.config/kanshi/config)
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "stdout",
        conflicts_with = "output"
    )]
    config: Option<PathBuf>,
    /// Print generated profile to stdout (raw mode, no config parsing/upsert)
    #[arg(long, conflicts_with = "output")]
    stdout: bool,
    /// Write generated profile to a file path (raw mode, no config parsing/upsert)
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,
}

fn read_input(input_json: &str) -> Result<Vec<u8>> {
    match input_json {
        "-" => {
            let mut input = Vec::new();
            io::stdin()
                .read_to_end(&mut input)
                .wrap_err("failed to read JSON from stdin")?;
            Ok(input)
        }
        path => fs::read(path).wrap_err_with(|| format!("failed to read input JSON from `{path}`")),
    }
}

fn write_raw_output(kanshi: &str, output: Option<&PathBuf>) -> Result<()> {
    match output {
        None => {
            print!("{kanshi}");
            Ok(())
        }
        Some(path) => fs::write(path, kanshi)
            .wrap_err_with(|| format!("failed to write generated profile to `{}`", path.display())),
    }
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Arguments::parse();
    let kanshi = if let Some(input_json) = args.input_json.as_deref() {
        let raw_json = read_input(input_json)?;
        generate_profile_from_slice(&args.name, &raw_json)
            .wrap_err("failed to generate kanshi profile from JSON input")?
    } else {
        let outputs = collect_outputs_wayland()
            .wrap_err("failed to collect output state from Wayland protocol")?;
        generate_profile_from_outputs(&args.name, &outputs)
            .wrap_err("failed to generate kanshi profile from Wayland state")?
    };

    if args.stdout || args.output.is_some() {
        write_raw_output(&kanshi, args.output.as_ref())?;
        return Ok(());
    }

    let config_path = match args.config {
        Some(path) => path,
        None => resolve_default_kanshi_config_path()
            .wrap_err("failed to resolve default kanshi config path")?,
    };
    upsert_profile_in_file(&config_path, &args.name, &kanshi).wrap_err_with(|| {
        format!(
            "failed to update kanshi config at `{}`",
            config_path.display()
        )
    })?;
    Ok(())
}
