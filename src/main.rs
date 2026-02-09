use std::{
    fs,
    io::{self, Read as _},
};

use clap::Parser;
use color_eyre::{Result, eyre::Context as _};
use kanshi_generate::{capture_wlr_randr_json, generate_profile_from_slice};

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
    /// Write output to a file path or '-' for stdout
    #[arg(long, value_name = "PATH")]
    output: Option<String>,
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

fn write_output(kanshi: &str, output: Option<&str>) -> Result<()> {
    match output {
        Some("-") | None => {
            print!("{kanshi}");
            Ok(())
        }
        Some(path) => fs::write(path, kanshi)
            .wrap_err_with(|| format!("failed to write generated profile to `{path}`")),
    }
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Arguments::parse();
    let wlr_randr_json = read_input(args.input_json.as_deref())?;
    let kanshi = generate_profile_from_slice(&args.name, &wlr_randr_json)
        .wrap_err("failed to generate kanshi profile")?;
    write_output(&kanshi, args.output.as_deref())?;
    Ok(())
}
