use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthStr;
use zellij_tile::prelude::*;

// ---- ANSI colors ------------------------------------------------------------
const RESET: &str = "\x1b[0m";
const ACTIVE_BG: &str = "\x1b[48;5;238m"; // selected tab block background (dark gray)
const FG_NORMAL: &str = "\x1b[38;5;252m"; // normal text on the panel
const DIM: &str = "\x1b[38;5;244m"; // descriptions (gray)
const BOLD: &str = "\x1b[1m";

// Row markers
const TAB_ACTIVE: &str = "●";
const TAB_INACTIVE: &str = "o";
const PANE_FOCUSED: &str = "▸";
const PANE_UNFOCUSED: &str = "·";

/// What clicking a rendered row should do.
#[derive(Clone, Copy)]
enum Action {
    SwitchTab(u32), // 1-based tab index
    FocusPane(u32), // terminal pane id
}

#[derive(Default)]
struct State {
    tabs: Vec<TabInfo>,
    panes: PaneManifest,
    active_idx: usize, // 1-based

    // Custom, user-updatable descriptions (set via `zellij pipe`).
    tab_desc: HashMap<usize, String>, // key: 1-based tab index
    pane_desc: HashMap<u32, String>,  // key: terminal pane id

    // Live per-pane working directory (from CwdChanged events).
    cwd: HashMap<u32, PathBuf>, // key: terminal pane id

    // Click hit-testing: rendered row -> action.
    row_action: Vec<Option<Action>>,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _config: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
        subscribe(&[
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::Mouse,
            EventType::CwdChanged,
            EventType::PermissionRequestResult,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        let mut should_render = false;
        match event {
            Event::PermissionRequestResult(_) => {
                set_selectable(false);
                should_render = true;
            }
            Event::TabUpdate(tabs) => {
                self.active_idx = tabs
                    .iter()
                    .position(|t| t.active)
                    .map(|i| i + 1)
                    .unwrap_or(1);
                self.tabs = tabs;
                should_render = true;
            }
            Event::PaneUpdate(pm) => {
                self.panes = pm;
                should_render = true;
            }
            Event::CwdChanged(pane_id, new_cwd, _) => {
                if let PaneId::Terminal(id) = pane_id {
                    self.cwd.insert(id, new_cwd);
                    should_render = true;
                }
            }
            Event::Mouse(Mouse::LeftClick(row, _col)) => {
                let r = row as usize;
                if let Some(Some(action)) = self.row_action.get(r) {
                    match *action {
                        Action::SwitchTab(idx) => switch_tab_to(idx),
                        Action::FocusPane(id) => focus_terminal_pane(id, false, false),
                    }
                }
            }
            Event::Mouse(Mouse::ScrollUp(_)) => {
                let prev = self.active_idx.saturating_sub(1).max(1);
                switch_tab_to(prev as u32);
            }
            Event::Mouse(Mouse::ScrollDown(_)) => {
                let n = self.tabs.len().max(1);
                let next = (self.active_idx + 1).min(n);
                switch_tab_to(next as u32);
            }
            _ => {}
        }
        should_render
    }

    /// Receive updatable descriptions from the CLI:
    ///   zellij pipe --name tab_desc  --args "id=1"  -- "text"
    ///   zellij pipe --name pane_desc --args "id=42" -- "text"
    /// An empty payload clears the entry.
    fn pipe(&mut self, msg: PipeMessage) -> bool {
        let id_arg = msg.args.get("id").cloned();
        let text = msg.payload.unwrap_or_default();
        match msg.name.as_str() {
            "tab_desc" => {
                if let Some(id) = id_arg.and_then(|s| s.parse::<usize>().ok()) {
                    if text.trim().is_empty() {
                        self.tab_desc.remove(&id);
                    } else {
                        self.tab_desc.insert(id, text);
                    }
                    return true;
                }
            }
            "pane_desc" => {
                if let Some(id) = id_arg.and_then(|s| s.parse::<u32>().ok()) {
                    if text.trim().is_empty() {
                        self.pane_desc.remove(&id);
                    } else {
                        self.pane_desc.insert(id, text);
                    }
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn render(&mut self, rows: usize, cols: usize) {
        // Do not gate on a permission event: when permissions are pre-granted
        // via permissions.kdl, PermissionRequestResult may not fire, and gating
        // on it would render nothing. Without permission, TabUpdate never fires
        // and tabs stays empty anyway.
        if self.tabs.is_empty() || cols == 0 {
            return;
        }

        let plan = self.build_plan();

        let mut lines: Vec<String> = Vec::with_capacity(plan.len());
        self.row_action = Vec::with_capacity(plan.len());
        for row in plan {
            self.row_action.push(row.action);
            lines.push(row.render(cols));
        }
        lines.truncate(rows);
        self.row_action.truncate(rows);

        print!("{}", lines.join("\n"));
    }
}

/// A single rendered row and the click action it maps to.
struct Row {
    text: String, // already-styled content (without padding/reset handling of bg)
    gray: bool,   // description line (rendered dim)
    active: bool, // belongs to the active tab / focused pane (bg highlight)
    bold: bool,   // emphasize (tab title / focused pane title)
    action: Option<Action>,
}

impl Row {
    fn render(&self, cols: usize) -> String {
        let text = truncate(&self.text, cols);
        let padded = pad_to(&text, cols);
        // Only the selected tab block gets a background; everything else keeps
        // the terminal default (black). Text colors stay constant.
        let bg = if self.active { ACTIVE_BG } else { "" };
        let fg = if self.gray { DIM } else { FG_NORMAL };
        let b = if self.bold { BOLD } else { "" };
        format!("{}{}{}{}{}", bg, fg, b, padded, RESET)
    }
}

impl State {
    /// Build the full list of rows (tabs, their descriptions, nested panes).
    fn build_plan(&self) -> Vec<Row> {
        let mut plan: Vec<Row> = Vec::new();

        for (i, tab) in self.tabs.iter().enumerate() {
            let idx = i + 1;
            let tab_active = tab.active;

            // Tab title line: "● name" / "o name"
            let marker = if tab_active { TAB_ACTIVE } else { TAB_INACTIVE };
            plan.push(Row {
                text: format!("{} {}", marker, self.tab_name(tab)),
                gray: false,
                active: tab_active,
                bold: true,
                action: Some(Action::SwitchTab(idx as u32)),
            });

            // Tab description line (custom, else derived). Always shown.
            // Whole active-tab block shares the same background (active = tab_active).
            plan.push(Row {
                text: format!("  {}", self.tab_description(tab, idx)),
                gray: true,
                active: tab_active,
                bold: false,
                action: Some(Action::SwitchTab(idx as u32)),
            });

            // Panes nested under the tab.
            if let Some(panes) = self.panes.panes.get(&tab.position) {
                for p in panes.iter().filter(|p| !p.is_plugin) {
                    let focused = tab_active && p.is_focused;
                    // Focus within the active tab is shown by the marker + bold,
                    // not a different background.
                    let pmark = if focused {
                        PANE_FOCUSED
                    } else {
                        PANE_UNFOCUSED
                    };
                    plan.push(Row {
                        text: format!("  {} {}", pmark, self.pane_name(p)),
                        gray: false,
                        active: tab_active,
                        bold: focused,
                        action: Some(Action::FocusPane(p.id)),
                    });

                    if let Some(desc) = self.pane_description(p) {
                        plan.push(Row {
                            text: format!("      {}", desc),
                            gray: true,
                            active: tab_active,
                            bold: false,
                            action: Some(Action::FocusPane(p.id)),
                        });
                    }
                }
            }
        }
        plan
    }

    /// Tab name: use zellij's own tab name so it matches the built-in tab bar
    /// (unnamed tabs are "Tab #N"). Never falls back to "...".
    fn tab_name(&self, tab: &TabInfo) -> String {
        if tab.name.is_empty() {
            format!("Tab #{}", tab.position + 1)
        } else {
            tab.name.clone()
        }
    }

    /// Tab description: custom (via pipe) else derived pane count + flags.
    /// Tab description: custom (via pipe) -> focused pane cwd -> focused command.
    fn tab_description(&self, tab: &TabInfo, idx: usize) -> String {
        if let Some(d) = self.tab_desc.get(&idx) {
            return d.clone();
        }
        if let Some(p) = self.focused_pane(tab.position) {
            if let Some(cwd) = self.cwd.get(&p.id) {
                return shorten_path(cwd);
            }
            if let Some(cmd) = &p.terminal_command {
                return command_of(cmd);
            }
            if let Some(n) = meaningful_name(&p.title) {
                return n;
            }
        }
        String::new()
    }

    fn focused_pane<'a>(&'a self, pos: usize) -> Option<&'a PaneInfo> {
        self.panes
            .panes
            .get(&pos)?
            .iter()
            .find(|p| p.is_focused && !p.is_plugin)
    }

    /// Pane name: meaningful title -> command -> "pane {id}". Never "...".
    fn pane_name(&self, p: &PaneInfo) -> String {
        if let Some(n) = meaningful_name(&p.title) {
            return n;
        }
        if let Some(cmd) = &p.terminal_command {
            return command_of(cmd);
        }
        format!("pane {}", p.id)
    }

    /// Pane description: custom (via pipe) -> cwd -> terminal command.
    fn pane_description(&self, p: &PaneInfo) -> Option<String> {
        if let Some(d) = self.pane_desc.get(&p.id) {
            return Some(d.clone());
        }
        if let Some(cwd) = self.cwd.get(&p.id) {
            return Some(shorten_path(cwd));
        }
        p.terminal_command.clone()
    }
}

/// Return the title if it is a real title (not a zellij placeholder).
fn meaningful_name(title: &str) -> Option<String> {
    let t = title.trim();
    if t.is_empty() || t.starts_with("Pane #") || t.starts_with("Tab #") {
        None
    } else {
        Some(t.to_string())
    }
}

/// Compact a path to its last two components (e.g. "Projects/foo").
fn shorten_path(p: &Path) -> String {
    let comps: Vec<&str> = p
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .filter(|s| *s != "/")
        .collect();
    match comps.len() {
        0 => "/".to_string(),
        1 => comps[0].to_string(),
        n => format!("{}/{}", comps[n - 2], comps[n - 1]),
    }
}

/// Extract a command name from a command line: first token -> path basename.
fn command_of(cmd: &str) -> String {
    let first = cmd.split_whitespace().next().unwrap_or(cmd);
    let base = first.rsplit('/').next().unwrap_or(first);
    base.to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = ch.to_string().width();
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

fn pad_to(s: &str, cols: usize) -> String {
    let w = s.width();
    if w >= cols {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(cols - w))
    }
}
