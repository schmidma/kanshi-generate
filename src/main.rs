use std::{
    fs,
    io::{self, Read as _},
    path::PathBuf,
};

use clap::Parser;
use color_eyre::{Result, eyre::Context as _};
use kanshi_generate::{
    capture_wlr_randr_json, generate_profile_from_slice, resolve_default_kanshi_config_path,
    upsert_profile_in_file,
};

#[derive(Debug, Parser)]
#[command(
    about = "Generate a kanshi profile from wlr-randr output",
    version,
    author
)]
struct Arguments {
    /// Profile name
    name: String,
    /// Read JSON from a file path or '-' for stdin instead of calling wlr-randr
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

fn read_input(input_json: Option<&str>) -> Result<Vec<u8>> {
    match input_json {
        Some("-") => {
            let mut input = Vec::new();
            io::stdin()
                .read_to_end(&mut input)
                .wrap_err("failed to read JSON from stdin")?;
            Ok(input)
        }
        Some(path) => {
            fs::read(path).wrap_err_with(|| format!("failed to read input JSON from `{path}`"))
        }
        None => capture_wlr_randr_json().wrap_err("failed to collect data from wlr-randr"),
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
    let wlr_randr_json = read_input(args.input_json.as_deref())?;
    let kanshi = generate_profile_from_slice(&args.name, &wlr_randr_json)
        .wrap_err("failed to generate kanshi profile")?;

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
