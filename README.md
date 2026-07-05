# zellij-tab-sidebar

A vertical tab sidebar plugin for [zellij](https://zellij.dev/). Each tab shows
three fields — **title**, **status** and **description** — all of which can be
updated live from scripts or AI agents, via `zellij pipe` (push) or periodic
commands (pull, starship-style).

```
▎1 zellij-sidebar
▎  ● running
▎  polishing the design and
▎  fixing the render bug
 2 dotfiles
   ● idle
 3 server
   ● error
   tests failed on ci
```

| Field | Default | Override | Rendering |
|---|---|---|---|
| **title** | zellij tab name | `ZELLIJ_SIDEBAR_TITLE_COMMAND` / pipe `tab_title` | bold, one line |
| **status** | `running` \| `idle` \| `error` (derived from command panes) | `ZELLIJ_SIDEBAR_STATUS_COMMAND` / pipe `tab_status` | `● text`, colored |
| **description** | empty (hidden) | `ZELLIJ_SIDEBAR_DESCRIPTION_COMMAND` / pipe `tab_desc` | dim, word-wrapped, max 2 lines |

Precedence per field: **pipe (manual) → command (auto) → default**.

Plus: theme-aware colors, accent bar + highlight on the active tab, click to
switch, scroll to move between tabs, overflow scrolling that follows the
active tab.

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
cargo build --release --target wasm32-wasip1
cp target/wasm32-wasip1/release/zellij-tab-sidebar.wasm ~/.config/zellij/plugins/
```

## Usage

Create a layout that places the plugin in a narrow left pane:

```kdl
// ~/.config/zellij/layouts/vertical-sidebar.kdl
layout {
    pane split_direction="vertical" {
        pane size=26 borderless=true {
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

## Updating fields via `zellij pipe` (push)

Any process inside a pane can update its own tab. An empty payload clears the
override (falling back to command/default).

```sh
# Target the tab containing MY pane (recommended for agents):
zellij pipe --name tab_title  --args "pane_id=$ZELLIJ_PANE_ID" -- "release build"
zellij pipe --name tab_desc   --args "pane_id=$ZELLIJ_PANE_ID" -- "compiling, then running the test suite"
zellij pipe --name tab_status --args "pane_id=$ZELLIJ_PANE_ID" -- "running"

# Explicit tab (1-based) / active tab:
zellij pipe --name tab_status --args "tab=2" -- "red:tests failed"
zellij pipe --name tab_desc -- "reviewing PR"

# Clear:
zellij pipe --name tab_status --args "pane_id=$ZELLIJ_PANE_ID" -- ""
```

`$ZELLIJ_PANE_ID` is set by zellij in every pane, so an agent can always label
the tab it lives in without knowing anything else about the layout.

### Status colors

The status color is picked from the leading keyword:

- green: `running`, `busy`, `working`, `active`, `ok`, `success`
- red: `error`, `failed`, `fail`, `crashed`
- yellow: `waiting`, `warn`, `warning`, `pending`, `blocked`
- dim: `idle`, `done`, `stopped`, `exited`

…or set explicitly with a `color:` prefix:
`green:` `red:` `yellow:` `blue:` `magenta:` `cyan:` `orange:` `dim:` `normal:`
(e.g. `orange:rate-limited`).

## Field commands (pull, starship-style)

Each field can also be produced by a command executed per tab on an interval;
the first line of stdout becomes the value (description may use more — it is
word-wrapped to two lines). Empty output or a non-zero exit means "no value",
falling back to the default. Pipe-set values always win.

Enable via plugin config:

```kdl
plugin location="file:~/.config/zellij/plugins/zellij-tab-sidebar.wasm" {
    title_command       "basename \"$PWD\""
    description_command "cat .agent-status 2>/dev/null"
    status_command      "test -f .lock && echo yellow:locked || echo idle"
    interval "2"  // seconds (default 2)
}
```

…or via environment variables set **before starting zellij** (read from the
zellij server's environment, not from individual panes):

```sh
export ZELLIJ_SIDEBAR_TITLE_COMMAND='basename "$PWD"'
export ZELLIJ_SIDEBAR_DESCRIPTION_COMMAND='cat .agent-status 2>/dev/null || git branch --show-current 2>/dev/null'
export ZELLIJ_SIDEBAR_STATUS_COMMAND='cat .agent-state 2>/dev/null'
zellij
```

Commands run with:

- **cwd** — the focused pane's working directory of that tab
- `$ZELLIJ_TAB_POSITION` — 1-based tab index
- `$ZELLIJ_TAB_NAME` — the tab's name
- `$ZELLIJ_FOCUSED_PANE_ID` — the focused pane's id

An agent then only needs to write one-liners to files (e.g. `.agent-status`) in
its repo — no zellij integration required at all.

## Agent integrations (contrib)

Ready-made integrations that mirror an AI agent's state into the sidebar
automatically — description = the task it was asked to do, status =
running / idle — with no LLM cooperation needed.

### pi

Symlink the extension into pi's global extensions dir:

```sh
ln -s /path/to/contrib/pi-extension.ts ~/.pi/agent/extensions/zellij-sidebar.ts
```

It reacts to `before_agent_start` (description ← prompt, status running),
`agent_end` (idle) and `session_shutdown` (clears overrides). No-op outside
zellij.

### Claude Code

`contrib/claude-code-hook.sh` does the same via Claude Code hooks (plus
`yellow:waiting` when it needs your input). Wire it up in
`~/.claude/settings.json` (requires `jq`):

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "matcher": "", "hooks": [{ "type": "command",
      "command": "sh /path/to/contrib/claude-code-hook.sh prompt" }] }],
    "PostToolUse": [{ "matcher": "", "hooks": [{ "type": "command",
      "command": "sh /path/to/contrib/claude-code-hook.sh post-tool" }] }],
    "Stop": [{ "matcher": "", "hooks": [{ "type": "command",
      "command": "sh /path/to/contrib/claude-code-hook.sh stop" }] }],
    "Notification": [{ "matcher": "", "hooks": [{ "type": "command",
      "command": "sh /path/to/contrib/claude-code-hook.sh notification" }] }]
  }
}
```

Each tab running an agent then shows what it was asked to do and whether it's
working, waiting for you, or done.

## Permissions

The plugin requests `ReadApplicationState` (to read tabs/panes),
`ChangeApplicationState` (to switch tabs on click) and `RunCommands` (to run
the field commands). On first run, focus the sidebar pane and press `y` to
grant.

## Interaction

| Action | Effect |
|--------|--------|
| Left click on a tab | Switch to that tab |
| Scroll up / down | Switch to the previous / next tab |

## License

MIT
