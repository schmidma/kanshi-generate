# kanshi-generate

A small CLI tool that converts the output of [`wlr-randr`](https://gitlab.freedesktop.org/emersion/wlr-randr) into a [kanshi](https://gitlab.freedesktop.org/emersion/kanshi) profile configuration.

## Usage

```bash
kanshi-generate <profile-name> >> ~/.config/kanshi/config
```

This will create a kanshi profile named `<profile-name>` using the currently connected Wayland outputs.

## Installation

```bash
cargo install --git https://github.com/schmidma/kanshi-generate
```

## License

MIT
