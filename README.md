# kanshi-generate

[![CI](https://github.com/schmidma/kanshi-generate/actions/workflows/ci.yml/badge.svg)](https://github.com/schmidma/kanshi-generate/actions)

A small CLI tool that converts the output of [`wlr-randr`](https://gitlab.freedesktop.org/emersion/wlr-randr) into a [kanshi](https://gitlab.freedesktop.org/emersion/kanshi) profile configuration.

## Usage

```bash
kanshi-generate <profile-name>
```

By default this updates your kanshi config in-place:

- Replaces profile `<profile-name>` if it exists exactly once.
- Appends profile `<profile-name>` if it does not exist yet.
- Fails without writing if duplicate profile blocks with the same name exist.

Default config path:

- `$XDG_CONFIG_HOME/kanshi/config` (if `XDG_CONFIG_HOME` is set)
- otherwise `$HOME/.config/kanshi/config`

### Options

```text
Usage: kanshi-generate [OPTIONS] <NAME>

Arguments:
  <NAME>  Profile name

Options:
      --config <PATH>      Override kanshi config path for in-place profile upsert
      --stdout             Print generated profile to stdout (raw mode, no config parsing/upsert)
      --input-json <PATH>  Read JSON from a file path or '-' for stdin instead of calling wlr-randr
      --output <PATH>      Write generated profile to file (raw mode, no config parsing/upsert)
  -h, --help               Print help
  -V, --version            Print version
```

Examples:

```bash
# Use live compositor state (default behavior)
kanshi-generate docked

# Override config path for in-place overwrite/append
kanshi-generate docked --config ~/.config/kanshi/config

# Print generated profile only (no config edit)
kanshi-generate docked --stdout

# Use previously captured JSON
wlr-randr --json > outputs.json
kanshi-generate docked --input-json outputs.json

# Pipe JSON over stdin
wlr-randr --json | kanshi-generate docked --input-json -

# Write generated profile directly to a file (no config parse/merge)
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
- If your config contains duplicate profile names, the command fails to avoid ambiguous overwrites.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## License

MIT
