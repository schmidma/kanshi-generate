# kanshi-generate

[![CI](https://github.com/schmidma/kanshi-generate/actions/workflows/ci.yml/badge.svg)](https://github.com/schmidma/kanshi-generate/actions)

A small CLI tool that converts the output of [`wlr-randr`](https://gitlab.freedesktop.org/emersion/wlr-randr) into a [kanshi](https://gitlab.freedesktop.org/emersion/kanshi) profile configuration.

## Usage

```bash
kanshi-generate <profile-name> >> ~/.config/kanshi/config
```

This will create a kanshi profile named `<profile-name>` using the currently connected Wayland outputs.

### Options

```text
Usage: kanshi-generate [OPTIONS] <NAME>

Arguments:
  <NAME>  Profile name

Options:
      --input-json <PATH>  Read JSON from a file path or '-' for stdin instead of calling wlr-randr
      --output <PATH>      Write output to a file path or '-' for stdout
  -h, --help               Print help
  -V, --version            Print version
```

Examples:

```bash
# Use live compositor state (default behavior)
kanshi-generate docked

# Use previously captured JSON
wlr-randr --json > outputs.json
kanshi-generate docked --input-json outputs.json

# Pipe JSON over stdin
wlr-randr --json | kanshi-generate docked --input-json -

# Write directly to a file (overwrites target file)
kanshi-generate docked --output ~/.config/kanshi/generated-profile.conf
```

## Installation

```bash
cargo install --git https://github.com/schmidma/kanshi-generate --locked
```

## Troubleshooting

- If `wlr-randr` cannot reach your compositor, the command now exits with the original stderr from `wlr-randr`.
- If a monitor is enabled but missing mode/position/scale data, the command fails fast with an explicit output-specific error.
- Disabled outputs may not include `position`/`scale` in JSON; this is handled automatically.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## License

MIT
