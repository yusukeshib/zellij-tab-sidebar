use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;
use zellij_tile::prelude::*;

// ---- ANSI helpers -----------------------------------------------------------
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const FIELD_SEP: char = '\u{1f}'; // unit separator, used by the env probe

fn fg(c: PaletteColor) -> String {
    match c {
        PaletteColor::Rgb((r, g, b)) => format!("\x1b[38;2;{};{};{}m", r, g, b),
        PaletteColor::EightBit(n) => format!("\x1b[38;5;{}m", n),
    }
}

fn bg(c: PaletteColor) -> String {
    match c {
        PaletteColor::Rgb((r, g, b)) => format!("\x1b[48;2;{};{};{}m", r, g, b),
        PaletteColor::EightBit(n) => format!("\x1b[48;5;{}m", n),
    }
}

/// Approximate an xterm 256-color index as RGB.
fn eightbit_to_rgb(n: u8) -> (u8, u8, u8) {
    match n {
        // Standard + bright 16 colors.
        0 => (0, 0, 0),
        1 => (128, 0, 0),
        2 => (0, 128, 0),
        3 => (128, 128, 0),
        4 => (0, 0, 128),
        5 => (128, 0, 128),
        6 => (0, 128, 128),
        7 => (192, 192, 192),
        8 => (128, 128, 128),
        9 => (255, 0, 0),
        10 => (0, 255, 0),
        11 => (255, 255, 0),
        12 => (0, 0, 255),
        13 => (255, 0, 255),
        14 => (0, 255, 255),
        15 => (255, 255, 255),
        // 6x6x6 color cube.
        16..=231 => {
            let i = n - 16;
            let step = |c: u8| if c == 0 { 0 } else { 55 + 40 * c };
            (step(i / 36), step((i / 6) % 6), step(i % 6))
        }
        // Grayscale ramp.
        232..=255 => {
            let v = 8 + (n - 232) * 10;
            (v, v, v)
        }
    }
}

/// Darken a color toward black by `factor` (0.0 = black, 1.0 = unchanged).
fn darken(c: PaletteColor, factor: f32) -> PaletteColor {
    let (r, g, b) = match c {
        PaletteColor::Rgb(rgb) => rgb,
        PaletteColor::EightBit(n) => eightbit_to_rgb(n),
    };
    let f = factor.clamp(0.0, 1.0);
    let s = |v: u8| (v as f32 * f).round() as u8;
    PaletteColor::Rgb((s(r), s(g), s(b)))
}

// ---- Fields ------------------------------------------------------------------
/// The three per-tab fields shown in the sidebar. Each one can be overridden
/// by a pipe message or produced by a command (config / env var).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Field {
    Title,
    Description,
    Status,
}

const FIELDS: [Field; 3] = [Field::Title, Field::Description, Field::Status];

impl Field {
    fn key(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::Description => "description",
            Field::Status => "status",
        }
    }

    fn config_key(self) -> &'static str {
        match self {
            Field::Title => "title_command",
            Field::Description => "description_command",
            Field::Status => "status_command",
        }
    }

    fn env_var(self) -> &'static str {
        match self {
            Field::Title => "ZELLIJ_SIDEBAR_TITLE_COMMAND",
            Field::Description => "ZELLIJ_SIDEBAR_DESCRIPTION_COMMAND",
            Field::Status => "ZELLIJ_SIDEBAR_STATUS_COMMAND",
        }
    }

    /// Accepted `zellij pipe --name <n>` names for this field.
    fn matches_pipe(self, name: &str) -> bool {
        match self {
            Field::Title => matches!(name, "tab_title" | "title"),
            Field::Description => matches!(name, "tab_desc" | "tab_description" | "description"),
            Field::Status => matches!(name, "tab_status" | "status"),
        }
    }
}

/// Status color, resolved against the theme at render time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatusColor {
    Green,
    Red,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Orange,
    Dim,
    Normal,
}

impl StatusColor {
    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "green" => Self::Green,
            "red" => Self::Red,
            "yellow" => Self::Yellow,
            "blue" => Self::Blue,
            "magenta" => Self::Magenta,
            "cyan" => Self::Cyan,
            "orange" => Self::Orange,
            "dim" | "gray" | "grey" => Self::Dim,
            "normal" | "default" => Self::Normal,
            _ => return None,
        })
    }

    /// Auto-detect a color from a status keyword.
    fn from_keyword(s: &str) -> Self {
        let w = s
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match w.as_str() {
            "running" | "busy" | "working" | "active" | "ok" | "success" => Self::Green,
            "error" | "failed" | "fail" | "crashed" => Self::Red,
            "waiting" | "warn" | "warning" | "pending" | "blocked" => Self::Yellow,
            "idle" | "done" | "stopped" | "exited" => Self::Dim,
            _ => Self::Normal,
        }
    }

    fn sgr(self, colors: &Styling, normal_fg: &str) -> String {
        match self {
            Self::Green => fg(colors.exit_code_success.base),
            Self::Red => fg(colors.exit_code_error.base),
            Self::Yellow => fg(PaletteColor::EightBit(3)),
            Self::Blue => fg(PaletteColor::EightBit(4)),
            Self::Magenta => fg(PaletteColor::EightBit(5)),
            Self::Cyan => fg(PaletteColor::EightBit(6)),
            Self::Orange => fg(PaletteColor::EightBit(208)),
            Self::Dim => format!("{}{}", DIM, normal_fg),
            Self::Normal => normal_fg.to_string(),
        }
    }
}

/// Parse a status value: an optional `color:` prefix wins, otherwise the
/// leading keyword picks the color (running -> green, error -> red, ...).
fn parse_status(s: &str) -> (String, StatusColor) {
    if let Some((c, rest)) = s.split_once(':') {
        if let Some(color) = StatusColor::from_name(c.trim()) {
            return (rest.trim().to_string(), color);
        }
    }
    (s.to_string(), StatusColor::from_keyword(s))
}

// ---- Cross-instance persistence ---------------------------------------------
// `default_layout` injects the sidebar into every tab, so there is one plugin
// instance PER TAB, each with its own memory. A `zellij pipe` override only
// reaches the instances alive at pipe time, so instances created later (e.g.
// when you open a new tab) never saw past overrides and render those tabs blank.
//
// Fix: persist overrides to `/cache` — the only plugin folder zellij shares
// across instances (`/data` maps to a per-instance dir keyed by
// `<plugin_id>-<client_id>`, so it is NOT shared) — and reload it on the timer.
// `/cache` persists across sessions and is shared between concurrent sessions,
// so the file is scoped by session name (pane ids restart per session).
// Keyed by pane id: stable across tab reorder/close, and identical across
// instances (they all receive the same PaneManifest).
//
// Two kinds share the same on-disk format under different files:
//   "pipe" — explicit `zellij pipe` overrides (highest precedence)
//   "cmd"  — per-field command output. Shared too (not just per-instance) so
//            every tab's sidebar renders the SAME description for a tab,
//            instead of each instance computing its own and disagreeing.
fn state_path(session: &str, kind: &str) -> String {
    format!("/cache/{}-state-{}.tsv", kind, session)
}

fn read_state(session: &str, kind: &str) -> HashMap<(Field, u32), String> {
    let mut map = HashMap::new();
    let Ok(content) = std::fs::read_to_string(state_path(session, kind)) else {
        return map;
    };
    for line in content.lines() {
        // Format: <pane_id>\t<field_key>\t<value>
        let mut it = line.splitn(3, '\t');
        let (Some(pid), Some(key), Some(val)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let (Ok(pid), Some(field)) = (
            pid.parse::<u32>(),
            FIELDS.iter().copied().find(|f| f.key() == key),
        ) else {
            continue;
        };
        map.insert((field, pid), val.to_string());
    }
    map
}

fn write_state(session: &str, kind: &str, map: &HashMap<(Field, u32), String>) {
    let mut s = String::new();
    for ((field, pid), val) in map {
        // Values are single-line records; strip separators defensively.
        let val = val.replace(['\n', '\r', '\t'], " ");
        s.push_str(&pid.to_string());
        s.push('\t');
        s.push_str(field.key());
        s.push('\t');
        s.push_str(&val);
        s.push('\n');
    }
    let _ = std::fs::write(state_path(session, kind), s);
}

// ---- State -------------------------------------------------------------------

/// Timer cadence: pipe-state sync every tick, commands every `interval`.
const TICK_SECS: f64 = 2.0;
#[derive(Default)]
struct State {
    tabs: Vec<TabInfo>,
    panes: PaneManifest,
    active_idx: usize, // 1-based
    style: Style,
    // Session name (from ModeUpdate) — scopes the shared override file.
    session: Option<String>,

    // Per-field commands (config `*_command`, or env probe).
    commands: HashMap<Field, String>,
    interval: f64,
    started: bool,       // engine kicked off
    timer_running: bool, // refresh timer scheduled
    ticks_left: u32,     // timer ticks until the next command run

    // Overrides set via `zellij pipe` (highest precedence), keyed by pane id and
    // persisted to `/cache` so all sidebar instances (one per tab) stay in sync.
    pipe_vals: HashMap<(Field, u32), String>,
    // Values produced by the per-field commands. Keyed by pane id (like
    // `pipe_vals`) and persisted to the shared `/cache` file so every tab's
    // sidebar instance renders the same command output for a given tab.
    cmd_vals: HashMap<(Field, u32), String>,
    // This instance's own plugin pane id (from `get_plugin_ids`), used to find
    // which tab it lives in so it only runs commands for that tab.
    plugin_id: u32,

    // Live per-pane working directory (cwd for the field commands).
    cwd: HashMap<u32, PathBuf>, // key: terminal pane id

    // Click hit-testing: rendered row -> 1-based tab index.
    row_action: Vec<Option<u32>>,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, config: BTreeMap<String, String>) {
        // Remember our own plugin pane id so we can find which tab we live in.
        self.plugin_id = get_plugin_ids().plugin_id;
        for f in FIELDS {
            if let Some(cmd) = config.get(f.config_key()) {
                let cmd = cmd.trim();
                if !cmd.is_empty() {
                    self.commands.insert(f, cmd.to_string());
                }
            }
        }
        self.interval = config
            .get("interval")
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(2.0)
            .max(0.5);
        // Overrides published by sibling instances are loaded once ModeUpdate
        // delivers the session name (the shared file is session-scoped).
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::RunCommands,
            PermissionType::ReadPaneContents,
        ]);
        subscribe(&[
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::ModeUpdate,
            EventType::Mouse,
            EventType::CwdChanged,
            EventType::Timer,
            EventType::RunCommandResult,
            EventType::PermissionRequestResult,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        let mut should_render = false;
        match event {
            Event::PermissionRequestResult(_) => {
                set_selectable(false);
                self.start_engine();
                should_render = true;
            }
            Event::ModeUpdate(mode_info) => {
                if self.style != mode_info.style {
                    self.style = mode_info.style;
                    should_render = true;
                }
                if self.session != mode_info.session_name {
                    self.session = mode_info.session_name.clone();
                    // First chance to load overrides published by siblings.
                    should_render = self.reload_shared() || should_render;
                }
            }
            Event::TabUpdate(tabs) => {
                self.active_idx = tabs
                    .iter()
                    .position(|t| t.active)
                    .map(|i| i + 1)
                    .unwrap_or(1);
                self.tabs = tabs;
                // TabUpdate only fires once permissions are granted; kick the
                // engine here too in case PermissionRequestResult never fired
                // (pre-granted permissions).
                self.start_engine();
                should_render = true;
            }
            Event::Timer(_) => {
                self.timer_running = false;
                // Pick up overrides written by other tabs' sidebar instances.
                should_render = self.reload_shared();
                // The timer ticks fast (cross-instance sync must be snappy);
                // the field commands only run every `interval` seconds.
                self.ticks_left = self.ticks_left.saturating_sub(1);
                if self.ticks_left == 0 {
                    self.spawn_commands();
                    self.ticks_left = (self.interval / TICK_SECS).ceil().max(1.0) as u32;
                }
                self.schedule_timer();
            }
            Event::RunCommandResult(exit_code, stdout, _stderr, ctx) => {
                should_render = self.handle_command_result(exit_code, stdout, ctx);
            }
            Event::PaneUpdate(pm) => {
                self.panes = pm;
                // Pane ids are recycled within a session: once a pane closes,
                // its id can be handed to a brand-new pane in another tab. If a
                // stale override lingered for the dead id, that unrelated tab
                // would suddenly render the old description. Drop overrides for
                // panes that no longer exist so descriptions stay attached to
                // the right tab.
                self.prune_dead_panes();
                should_render = true;
            }
            Event::CwdChanged(PaneId::Terminal(id), new_cwd, _) => {
                self.cwd.insert(id, new_cwd);
                should_render = true;
            }
            Event::Mouse(Mouse::LeftClick(row, _col)) => {
                let r = row as usize;
                if let Some(Some(idx)) = self.row_action.get(r) {
                    switch_tab_to(*idx);
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

    /// Set a field from the CLI (e.g. from an agent running in a pane):
    ///
    ///   zellij pipe --name tab_title  --args "pane_id=$ZELLIJ_PANE_ID" -- "my build"
    ///   zellij pipe --name tab_desc   --args "pane_id=$ZELLIJ_PANE_ID" -- "compiling the release"
    ///   zellij pipe --name tab_status --args "pane_id=$ZELLIJ_PANE_ID" -- "running"
    ///   zellij pipe --name tab_status --args "tab=2" -- "red:tests failed"
    ///   zellij pipe --name tab_desc -- "reviewing PR"   # no target -> active tab
    ///
    /// An empty payload clears the override (falling back to command/default).
    fn pipe(&mut self, msg: PipeMessage) -> bool {
        let Some(field) = FIELDS.iter().copied().find(|f| f.matches_pipe(&msg.name)) else {
            return false;
        };
        let text = msg.payload.unwrap_or_default();

        // Resolve the target pane id: explicit `pane_id`, else the focused (or
        // first) pane of the target tab (`tab=N`, or the active tab).
        let pane_id: Option<u32> =
            if let Some(pid) = msg.args.get("pane_id").and_then(|s| s.parse::<u32>().ok()) {
                Some(pid)
            } else {
                let pos = if let Some(t) = msg.args.get("tab").and_then(|s| s.parse::<usize>().ok())
                {
                    t.checked_sub(1)
                } else {
                    self.active_idx.checked_sub(1)
                };
                pos.and_then(|p| self.target_pane_id(p))
            };

        let Some(pid) = pane_id else {
            return false;
        };

        // Read-modify-write the shared file so concurrent writes from sibling
        // instances don't clobber each other.
        let session = self.session.clone().unwrap_or_default();
        let mut vals = read_state(&session, "pipe");
        if text.trim().is_empty() {
            vals.remove(&(field, pid));
        } else {
            vals.insert((field, pid), text.trim().to_string());
        }
        write_state(&session, "pipe", &vals);
        self.pipe_vals = vals;
        true
    }

    fn render(&mut self, rows: usize, cols: usize) {
        self.row_action.clear();
        if self.tabs.is_empty() || rows == 0 || cols == 0 {
            return;
        }

        let (lines, actions) = self.build_lines(cols);

        // Keep the active tab block visible when tabs overflow the pane.
        let offset = scroll_offset(&actions, self.active_idx as u32, lines.len(), rows);

        let visible: Vec<&String> = lines.iter().skip(offset).take(rows).collect();
        self.row_action = actions.into_iter().skip(offset).take(rows).collect();

        let mut out = String::new();
        for (i, l) in visible.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(l);
        }
        print!("{}", out);
    }
}

// ---- Engine (commands producing field values) ---------------------------------
impl State {
    /// Kick off the engine exactly once: probe the zellij server's environment
    /// for ZELLIJ_SIDEBAR_*_COMMAND vars (config takes precedence per field),
    /// and start the refresh timer if any command is already configured.
    fn start_engine(&mut self) {
        if self.started {
            return;
        }
        self.started = true;

        let mut ctx = BTreeMap::new();
        ctx.insert("kind".to_string(), "probe".to_string());
        let probe = format!(
            "printf '%s{sep}%s{sep}%s' \"${t}\" \"${d}\" \"${s}\"",
            sep = FIELD_SEP,
            t = Field::Title.env_var(),
            d = Field::Description.env_var(),
            s = Field::Status.env_var(),
        );
        run_command(&["sh", "-c", &probe], ctx);

        self.schedule_timer();
    }

    // Always runs (even with no field commands) so every instance periodically
    // reloads the shared override file and converges with its siblings.
    fn schedule_timer(&mut self) {
        if !self.timer_running {
            self.timer_running = true;
            set_timeout(self.interval.min(TICK_SECS));
        }
    }

    /// Run each configured field command once per tab, in the focused pane's
    /// cwd, with tab metadata exposed as environment variables.
    fn spawn_commands(&self) {
        if self.commands.is_empty() {
            return;
        }
        // Each tab has its own sidebar instance; only run commands for the tab
        // this instance lives in (results are shared via `/cache`, so every
        // instance renders the same value). This avoids N² command runs and,
        // more importantly, the disagreement that made a tab's description look
        // different depending on which tab you viewed it from. If we can't yet
        // tell which tab we're in (manifest not ready), fall back to all tabs.
        let own = self.own_tab();
        for tab in &self.tabs {
            if let Some(pos) = own {
                if tab.position != pos {
                    continue;
                }
            }
            // The pane a result is attributed to — same resolution `pipe_val`
            // uses to read it back (focused non-plugin, else first non-plugin).
            let Some(target) = self.target_pane_id(tab.position) else {
                continue;
            };
            let focused = self.focused_pane(tab.position);
            let cwd = focused
                .and_then(|p| self.cwd.get(&p.id).cloned())
                .unwrap_or_else(|| PathBuf::from("."));

            let mut env = BTreeMap::new();
            env.insert(
                "ZELLIJ_TAB_POSITION".to_string(),
                (tab.position + 1).to_string(),
            );
            env.insert("ZELLIJ_TAB_NAME".to_string(), tab.name.clone());
            if let Some(p) = focused {
                env.insert("ZELLIJ_FOCUSED_PANE_ID".to_string(), p.id.to_string());
                // Expose the focused pane's visible buffer so commands can
                // summarize what's actually happening in the tab.
                if let Ok(contents) = get_pane_scrollback(PaneId::Terminal(p.id), false) {
                    let text = pane_tail(&contents.viewport);
                    if !text.is_empty() {
                        env.insert("ZELLIJ_SIDEBAR_PANE_CONTENT".to_string(), text);
                    }
                }
            }

            for (field, cmd) in &self.commands {
                let mut ctx = BTreeMap::new();
                ctx.insert("kind".to_string(), field.key().to_string());
                ctx.insert("pane".to_string(), target.to_string());
                run_command_with_env_variables_and_cwd(
                    &["sh", "-c", cmd],
                    env.clone(),
                    cwd.clone(),
                    ctx,
                );
            }
        }
    }

    /// Handle probe / per-tab field command results.
    fn handle_command_result(
        &mut self,
        exit_code: Option<i32>,
        stdout: Vec<u8>,
        ctx: BTreeMap<String, String>,
    ) -> bool {
        let out = String::from_utf8_lossy(&stdout);
        match ctx.get("kind").map(|s| s.as_str()) {
            Some("probe") => {
                let parts: Vec<&str> = out.splitn(3, FIELD_SEP).collect();
                for (i, f) in FIELDS.iter().enumerate() {
                    let cmd = parts.get(i).map(|s| s.trim()).unwrap_or("");
                    // Config takes precedence over the env var.
                    if !cmd.is_empty() && !self.commands.contains_key(f) {
                        self.commands.insert(*f, cmd.to_string());
                    }
                }
                self.schedule_timer();
                false
            }
            Some(kind) => {
                let Some(field) = FIELDS.iter().copied().find(|f| f.key() == kind) else {
                    return false;
                };
                let Some(pid) = ctx.get("pane").and_then(|s| s.parse::<u32>().ok()) else {
                    return false;
                };
                // Description keeps its full output (it may wrap to two lines);
                // title/status only use the first line.
                let value = if field == Field::Description {
                    out.trim().replace('\n', " ")
                } else {
                    out.lines().next().unwrap_or("").trim().to_string()
                };
                let key = (field, pid);
                // Read-modify-write the shared file so concurrent writes from
                // sibling instances (each owning a different tab) don't clobber
                // each other, then adopt the merged result.
                let session = self.session.clone().unwrap_or_default();
                let mut vals = read_state(&session, "cmd");
                if exit_code == Some(0) && !value.is_empty() {
                    vals.insert(key, value);
                } else {
                    vals.remove(&key);
                }
                let changed = vals != self.cmd_vals;
                if changed {
                    write_state(&session, "cmd", &vals);
                    self.cmd_vals = vals;
                }
                changed
            }
            _ => false,
        }
    }
}

// ---- Field values --------------------------------------------------------------
impl State {
    /// A pipe override for this tab: the value stored for the tab's focused
    /// pane, else any non-plugin pane in the tab. Works for inactive tabs too
    /// (the PaneManifest lists every tab's panes).
    /// Look up a tab's value in a pane-keyed store: the tab's focused pane
    /// first, else any non-plugin pane in the tab. Works for inactive tabs too
    /// (the PaneManifest lists every tab's panes).
    fn store_val<'a>(
        &self,
        store: &'a HashMap<(Field, u32), String>,
        field: Field,
        tab: &TabInfo,
    ) -> Option<&'a String> {
        let panes = self.panes.panes.get(&tab.position)?;
        if let Some(p) = panes.iter().find(|p| p.is_focused && !p.is_plugin) {
            if let Some(v) = store.get(&(field, p.id)) {
                return Some(v);
            }
        }
        panes
            .iter()
            .filter(|p| !p.is_plugin)
            .find_map(|p| store.get(&(field, p.id)))
    }

    fn pipe_val(&self, field: Field, tab: &TabInfo) -> Option<&String> {
        self.store_val(&self.pipe_vals, field, tab)
    }

    fn field_val(&self, field: Field, tab: &TabInfo) -> Option<&String> {
        self.pipe_val(field, tab)
            .or_else(|| self.store_val(&self.cmd_vals, field, tab))
    }

    /// The tab position this plugin instance lives in (its plugin pane), so it
    /// only runs field commands for its own tab. `None` until the manifest
    /// listing our plugin pane arrives.
    fn own_tab(&self) -> Option<usize> {
        self.panes.panes.iter().find_map(|(pos, panes)| {
            panes
                .iter()
                .any(|p| p.is_plugin && p.id == self.plugin_id)
                .then_some(*pos)
        })
    }

    /// Reload both shared override files; returns whether anything changed.
    fn reload_shared(&mut self) -> bool {
        let session = self.session.as_deref().unwrap_or_default();
        let mut changed = false;
        let pipe = read_state(session, "pipe");
        if pipe != self.pipe_vals {
            self.pipe_vals = pipe;
            changed = true;
        }
        let cmd = read_state(session, "cmd");
        if cmd != self.cmd_vals {
            self.cmd_vals = cmd;
            changed = true;
        }
        changed
    }

    /// Drop overrides for panes that no longer exist. Pane ids are recycled
    /// within a session, so a lingering entry for a closed pane would otherwise
    /// resurface on whatever new pane inherits its id — in an unrelated tab.
    fn prune_dead_panes(&mut self) {
        if self.panes.panes.is_empty() {
            return; // no manifest yet; don't wipe everything
        }
        let live: std::collections::HashSet<u32> = self
            .panes
            .panes
            .values()
            .flatten()
            .filter(|p| !p.is_plugin)
            .map(|p| p.id)
            .collect();
        let session = self.session.clone().unwrap_or_default();
        for (kind, map) in [("pipe", true), ("cmd", false)] {
            let store = if map { &mut self.pipe_vals } else { &mut self.cmd_vals };
            let before = store.len();
            store.retain(|(_, pid), _| live.contains(pid));
            if store.len() != before {
                write_state(&session, kind, store);
            }
        }
    }

    /// The pane a `tab=N` / active-tab pipe targets: focused, else first.
    fn target_pane_id(&self, pos: usize) -> Option<u32> {
        let panes = self.panes.panes.get(&pos)?;
        panes
            .iter()
            .find(|p| p.is_focused && !p.is_plugin)
            .or_else(|| panes.iter().find(|p| !p.is_plugin))
            .map(|p| p.id)
    }

    /// Title: pipe -> command -> zellij tab name.
    fn tab_title(&self, tab: &TabInfo) -> String {
        if let Some(t) = self.field_val(Field::Title, tab) {
            return t.clone();
        }
        if tab.name.is_empty() {
            format!("Tab #{}", tab.position + 1)
        } else {
            tab.name.clone()
        }
    }

    /// Description: pipe -> command -> empty (hidden).
    fn tab_description(&self, tab: &TabInfo) -> String {
        self.field_val(Field::Description, tab)
            .cloned()
            .unwrap_or_default()
    }

    /// Status: pipe -> command -> derived running|idle|error.
    fn tab_status(&self, tab: &TabInfo) -> (String, StatusColor) {
        if let Some(s) = self.field_val(Field::Status, tab) {
            return parse_status(s);
        }
        self.default_status(tab)
    }

    /// Derived status: a command pane still running -> "running"; a command
    /// pane that exited non-zero -> "error"; otherwise "idle".
    fn default_status(&self, tab: &TabInfo) -> (String, StatusColor) {
        if let Some(panes) = self.panes.panes.get(&tab.position) {
            let cmds: Vec<&PaneInfo> = panes
                .iter()
                .filter(|p| !p.is_plugin && p.terminal_command.is_some())
                .collect();
            if cmds.iter().any(|p| !p.exited) {
                return ("running".to_string(), StatusColor::Green);
            }
            if cmds
                .iter()
                .any(|p| p.exited && p.exit_status.unwrap_or(0) != 0)
            {
                return ("error".to_string(), StatusColor::Red);
            }
        }
        ("idle".to_string(), StatusColor::Dim)
    }

    fn focused_pane(&self, pos: usize) -> Option<&PaneInfo> {
        self.panes
            .panes
            .get(&pos)?
            .iter()
            .find(|p| p.is_focused && !p.is_plugin)
    }

}

// ---- Rendering ------------------------------------------------------------------
impl State {
    /// Build every line of the sidebar plus its click target.
    ///
    /// Per tab:
    ///   ▎1 title            (bold; accent bar + themed bg when active)
    ///   ▎  ● status         (colored)
    ///   ▎  description...   (dim, word-wrapped, max 2 lines, hidden if empty)
    fn build_lines(&self, cols: usize) -> (Vec<String>, Vec<Option<u32>>) {
        let colors = self.style.colors;
        let normal = colors.text_unselected;
        let selected = colors.list_selected;
        let accent = colors.frame_selected.base;

        let mut lines = Vec::new();
        let mut actions = Vec::new();

        for (i, tab) in self.tabs.iter().enumerate() {
            let idx = (i + 1) as u32;
            let active = tab.active;

            let (block_bg, base_fg) = if active {
                (bg(darken(selected.background, 0.6)), fg(selected.base))
            } else {
                (String::new(), fg(normal.base))
            };
            let bar = if active {
                format!("{}▎{}", fg(accent), RESET)
            } else {
                " ".to_string()
            };
            let push = |text: String, lines: &mut Vec<String>, actions: &mut Vec<Option<u32>>| {
                lines.push(text);
                actions.push(Some(idx));
            };

            // Title line: `▎1 title` (the index number is dimmed).
            let num = idx.to_string();
            let title = truncate(
                &format!("{} {}", num, self.tab_title(tab)),
                cols.saturating_sub(2),
            );
            let padded = pad_to(&title, cols.saturating_sub(2));
            // Split off the leading index digits (ASCII, byte len == char len).
            let (head, tail) = padded.split_at(num.len().min(padded.len()));
            push(
                format!(
                    "{bg}{bar}{bg} {dim}{fgc}{head}{reset}{bg}{bold}{fgc}{tail}{reset}",
                    bg = block_bg,
                    bar = bar,
                    dim = DIM,
                    bold = BOLD,
                    fgc = base_fg,
                    head = head,
                    tail = tail,
                    reset = RESET,
                ),
                &mut lines,
                &mut actions,
            );

            // Status line: `▎  ● status`
            let (status, color) = self.tab_status(tab);
            let status = truncate(&status, cols.saturating_sub(6));
            let sgr = color.sgr(&colors, &base_fg);
            push(
                format!(
                    "{bg}{bar}{bg}   {sgr}● {text}{reset}",
                    bg = block_bg,
                    bar = bar,
                    sgr = sgr,
                    text = pad_to(&status, cols.saturating_sub(6)),
                    reset = RESET,
                ),
                &mut lines,
                &mut actions,
            );

            // Description: dim, word-wrapped, up to 2 lines, hidden when empty.
            let desc = self.tab_description(tab);
            for l in wrap(&desc, cols.saturating_sub(4), 2) {
                push(
                    format!(
                        "{bg}{bar}{bg}   {dim}{fgc}{text}{reset}",
                        bg = block_bg,
                        bar = bar,
                        dim = DIM,
                        fgc = base_fg,
                        text = pad_to(&l, cols.saturating_sub(4)),
                        reset = RESET,
                    ),
                    &mut lines,
                    &mut actions,
                );
            }
        }

        (lines, actions)
    }
}

/// Scroll offset so that the active tab's block stays fully visible.
fn scroll_offset(actions: &[Option<u32>], active: u32, total: usize, rows: usize) -> usize {
    if total <= rows {
        return 0;
    }
    let Some(first) = actions.iter().position(|a| *a == Some(active)) else {
        return 0;
    };
    let last = actions
        .iter()
        .rposition(|a| *a == Some(active))
        .unwrap_or(first);
    if last >= rows {
        (last + 1).saturating_sub(rows).min(first)
    } else {
        0
    }
}

/// Greedy word-wrap to `width`, at most `max_lines` lines; the last line is
/// truncated with an ellipsis if the text doesn't fit. Empty text -> no lines.
fn wrap(s: &str, width: usize, max_lines: usize) -> Vec<String> {
    // Normalize whitespace so the "content dropped?" check below is exact.
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() || width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let sep = if cur.is_empty() { 0 } else { 1 };
        if cur.width() + sep + word.width() <= width {
            if sep == 1 {
                cur.push(' ');
            }
            cur.push_str(word);
        } else {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            // A word longer than the width is broken across lines (CJK text
            // has no spaces, so a whole description is one "word").
            let mut word = word;
            while lines.len() < max_lines && word.width() > width {
                let (head, rest) = split_at_width(word, width);
                lines.push(head.to_string());
                word = rest;
            }
            if lines.len() == max_lines {
                break;
            }
            cur = word.to_string();
        }
    }
    if !cur.is_empty() && lines.len() < max_lines {
        lines.push(cur);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
    }
    // If we dropped content, mark the last visible line.
    let shown: usize = lines.iter().map(|l| l.width()).sum();
    let sep_count = lines.len().saturating_sub(1);
    if shown + sep_count < s.width() {
        if let Some(last) = lines.last_mut() {
            *last = truncate(&format!("{}…", last), width);
        }
    }
    lines
}

/// Tail of a pane viewport, suitable for an env var: trailing blank lines
/// dropped, last 40 lines, capped to 4KB (cut at a char boundary).
fn pane_tail(viewport: &[String]) -> String {
    let lines: Vec<&str> = viewport.iter().map(|l| l.trim_end()).collect();
    let end = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);
    let start = end.saturating_sub(40);
    let text = lines[start..end].join("\n");
    // Byte cap: keep the tail, not the head.
    const MAX: usize = 4096;
    if text.len() <= MAX {
        return text;
    }
    let mut cut = text.len() - MAX;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    text[cut..].to_string()
}

/// Split at the last char boundary whose prefix display width fits in `max`.
fn split_at_width(s: &str, max: usize) -> (&str, &str) {
    let mut w = 0usize;
    for (i, ch) in s.char_indices() {
        let cw = ch.to_string().width();
        if w + cw > max {
            return s.split_at(i);
        }
        w += cw;
    }
    (s, "")
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
