# zellij-tab-sidebar

A vertical tab sidebar plugin for [zellij](https://zellij.dev/). Each tab is
rendered on its own rows as a **title line + description line**, separated by a
divider — making it easy to scan many tabs at a glance, tmux-style.

- **Two-line tabs** — `▎ 1 name` on top, a description line (pane count / status)
  below, with a `───` separator between tabs.
- **Never shows `...`** — the tab name falls back gracefully:
  explicit tab name → focused pane command → `shell`.
- **Active tab highlighting** — reverse background + bold on the current tab.
- **Mouse support** — click a tab to switch to it; scroll to move between tabs.
- **Live** — follows tab switches instantly (re-renders on `TabUpdate`).

## Requirements

- [zellij](https://zellij.dev/) v0.44+
- A [Rust](https://rustup.rs/) toolchain to build from source

## Install

### From a release

Reference the released wasm directly from a layout — zellij fetches and caches it:

```kdl
plugin location="https://github.com/yusukeshib/zellij-tab-sidebar/releases/latest/download/zellij-tab-sidebar.wasm"
```

Or download it into your plugins dir:

```sh
mkdir -p ~/.config/zellij/plugins
curl -fsSL -o ~/.config/zellij/plugins/zellij-tab-sidebar.wasm \
  https://github.com/yusukeshib/zellij-tab-sidebar/releases/latest/download/zellij-tab-sidebar.wasm
```

### From source

```sh
rustup target add wasm32-wasip1
cargo build --release
cp target/wasm32-wasip1/release/zellij-tab-sidebar.wasm ~/.config/zellij/plugins/
```

## Usage

Create a layout that places the plugin in a narrow left (or right) pane:

```kdl
// ~/.config/zellij/layouts/vertical-sidebar.kdl
layout {
    pane split_direction="vertical" {
        pane size=24 borderless=true {
            plugin location="file:~/.config/zellij/plugins/zellij-tab-sidebar.wasm"
        }
        pane borderless=true focus=true
    }
    pane size=1 borderless=true {
        plugin location="zellij:compact-bar"
    }
}
```

Launch a session with it:

```sh
zellij --new-session-with-layout vertical-sidebar
```

To always use it, set `default_layout "vertical-sidebar"` in your zellij config.

## Permissions

The plugin requests `ReadApplicationState` (to read tabs/panes) and
`ChangeApplicationState` (to switch tabs on click). On first run, focus the
sidebar pane and press `y` to grant.

## Interaction

| Action | Effect |
|--------|--------|
| Left click on a tab | Switch to that tab |
| Scroll up / down | Switch to the previous / next tab |

## License

MIT
