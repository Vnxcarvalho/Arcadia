//! A tmux look-alike for Claude Code's agent teams.
//!
//! Claude Code opens one pane per teammate by calling the `tmux` CLI. Inside an Alethe terminal
//! `tmux` resolves to the `alethe-tmux` shim, which forwards each call to the app's local listener
//! (`/tmux`). This module keeps the pane registry, answers the queries and turns the pane commands
//! into Alethe terminals. Off unless `ALETHE_TMUX_BRIDGE=1`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::pty::{self, PtySessions};

pub const OPEN_EVENT: &str = "tmux-pane-open";
pub const CLOSE_EVENT: &str = "tmux-pane-close";

const HEADLESS_COLS: u16 = 120;
const HEADLESS_ROWS: u16 = 30;
const TMUX_VERSION: &str = "tmux 3.4";
/// The first argument the `alethe-team` shim sends, so its calls are told apart from tmux's.
pub const TEAM_COMMAND: &str = "alethe-team";
pub const OPEN_MEMBER_EVENT: &str = "agent-team-open";
pub const SEND_MEMBER_EVENT: &str = "agent-team-send";
/// The longest task a lead can send to a member in one `alethe-team send`.
const MAX_TEAM_MESSAGE: usize = 4000;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// On when the Agent teams feature is switched on in Preferences, or `ALETHE_TMUX_BRIDGE=1` is
/// set for a run without the UI. Read when a terminal starts, so it applies to terminals opened
/// afterwards.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
        || std::env::var("ALETHE_TMUX_BRIDGE").is_ok_and(|value| value == "1")
}

#[tauri::command]
pub fn tmux_bridge_set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

#[derive(Deserialize, Debug, Default)]
pub struct Request {
    pub args: Vec<String>,
    /// `TMUX_PANE` of the calling terminal.
    #[serde(default)]
    pub pane: Option<String>,
}

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Reply {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Reply {
    fn ok(stdout: impl Into<String>) -> Self {
        Self {
            code: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    fn fail(stderr: impl Into<String>) -> Self {
        Self {
            code: 1,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }
}

/// Side effects a command asks for, run by the app after the registry has been updated.
#[derive(Debug, PartialEq)]
pub enum Effect {
    Open {
        pane_id: String,
        pty_id: String,
        leader_pty: String,
        title: String,
        cwd: Option<String>,
        command_line: String,
        /// What follows the executable, so a later run can relaunch the teammate.
        args: Vec<String>,
    },
    /// A lead ran `alethe-team open <role> [task]`: one more member for its team.
    OpenMember {
        leader_pty: String,
        role: String,
        task: String,
    },
    /// A lead ran `alethe-team send <role> <message>`: a new task for a member that is already open.
    SendMember {
        leader_pty: String,
        role: String,
        message: String,
    },
    Kill {
        pty_id: String,
    },
    Write {
        pty_id: String,
        data: String,
    },
}

struct Pane {
    window: String,
    pty_id: Option<String>,
    leader_pty: String,
    title: String,
}

#[derive(Default)]
struct State {
    next_pane: u32,
    next_window: u32,
    panes: HashMap<String, Pane>,
    windows: HashMap<String, Vec<String>>,
    pty_pane: HashMap<String, String>,
}

#[derive(Default)]
pub struct Bridge {
    state: Mutex<State>,
}

fn run_id() -> &'static str {
    static RUN_ID: OnceLock<String> = OnceLock::new();
    RUN_ID.get_or_init(|| nanoid::nanoid!(6))
}

fn bridge() -> &'static Bridge {
    static BRIDGE: OnceLock<Bridge> = OnceLock::new();
    BRIDGE.get_or_init(Bridge::default)
}

impl Bridge {
    /// The pane a terminal's PTY is, creating its window on first sight. The leader of a team is
    /// the terminal Claude runs in; its teammates join that window.
    fn register_leader(&self, pty_id: &str) -> (String, String) {
        let mut state = self.state.lock().expect("tmux bridge state");
        if let Some(pane_id) = state.pty_pane.get(pty_id).cloned() {
            let window = state.panes[&pane_id].window.clone();
            return (pane_id, window);
        }
        let window = format!("@{}", state.next_window);
        state.next_window += 1;
        let pane_id = format!("%{}", state.next_pane);
        state.next_pane += 1;
        state.pty_pane.insert(pty_id.to_string(), pane_id.clone());
        state.panes.insert(
            pane_id.clone(),
            Pane {
                window: window.clone(),
                pty_id: Some(pty_id.to_string()),
                leader_pty: pty_id.to_string(),
                title: String::new(),
            },
        );
        state.windows.insert(window.clone(), vec![pane_id.clone()]);
        (pane_id, window)
    }

    pub fn plan(&self, request: &Request) -> (Reply, Vec<Effect>) {
        let mut args: &[String] = &request.args;
        while matches!(args.first().map(String::as_str), Some("-L" | "-S")) && args.len() >= 2 {
            args = &args[2..];
        }
        let Some(sub) = args.first().map(String::as_str) else {
            return (Reply::ok(""), Vec::new());
        };
        let rest = &args[1..];
        let mut state = self.state.lock().expect("tmux bridge state");
        let caller = request.pane.clone().unwrap_or_else(|| "%0".to_string());

        match sub {
            // Not a tmux command: the helper an Antigravity lead runs to open a team member. Its
            // `pane` is the terminal's own PTY id (`ALETHE_TEAM_PTY`).
            TEAM_COMMAND => team_command(rest, request.pane.as_deref()),
            "-V" => (Reply::ok(format!("{TMUX_VERSION}\n")), Vec::new()),
            "display-message" => {
                let pane = resolve_pane(&state, flag(rest, "-t"), &caller);
                let format = rest.last().map(String::as_str).unwrap_or("");
                (
                    Reply::ok(format!("{}\n", expand(&state, format, &pane))),
                    Vec::new(),
                )
            }
            "list-panes" => {
                let window = resolve_window(&state, flag(rest, "-t"), &caller);
                let format = flag(rest, "-F").unwrap_or("#{pane_id}");
                let mut out = String::new();
                for pane in state.windows.get(&window).into_iter().flatten() {
                    out.push_str(&expand(&state, format, pane));
                    out.push('\n');
                }
                (Reply::ok(out), Vec::new())
            }
            "split-window" => {
                let anchor = resolve_pane(&state, flag(rest, "-t"), &caller);
                let Some(window) = state.panes.get(&anchor).map(|pane| pane.window.clone()) else {
                    return (Reply::fail(format!("can't find pane: {anchor}\n")), Vec::new());
                };
                let leader_pty = state.panes[&anchor].leader_pty.clone();
                let pane_id = format!("%{}", state.next_pane);
                state.next_pane += 1;
                state.panes.insert(
                    pane_id.clone(),
                    Pane {
                        window: window.clone(),
                        pty_id: None,
                        leader_pty,
                        title: String::new(),
                    },
                );
                state
                    .windows
                    .entry(window)
                    .or_default()
                    .push(pane_id.clone());
                let stdout = if rest.iter().any(|arg| arg == "-P") {
                    format!(
                        "{}\n",
                        expand(&state, flag(rest, "-F").unwrap_or("#{pane_id}"), &pane_id)
                    )
                } else {
                    String::new()
                };
                (Reply::ok(stdout), Vec::new())
            }
            "select-pane" => {
                if let Some(title) = flag(rest, "-T") {
                    let pane = resolve_pane(&state, flag(rest, "-t"), &caller);
                    if let Some(pane) = state.panes.get_mut(&pane) {
                        pane.title = title.to_string();
                    }
                }
                (Reply::ok(""), Vec::new())
            }
            "respawn-pane" => {
                let pane_id = resolve_pane(&state, flag(rest, "-t"), &caller);
                let Some(line) = rest
                    .iter()
                    .position(|arg| arg == "--")
                    .map(|index| rest[index + 1..].join(" "))
                    .filter(|line| !line.trim().is_empty())
                else {
                    return (Reply::fail("respawn-pane: no command\n"), Vec::new());
                };
                let Some(pane) = state.panes.get_mut(&pane_id) else {
                    return (Reply::fail(format!("can't find pane: {pane_id}\n")), Vec::new());
                };
                let (cwd, command_line) = translate_command(&line);
                let args = executable_args(&line);
                let existing = pane.pty_id.clone();
                // Pane numbers restart with every launch, but terminals are saved across them: the
                // run id keeps a new team from matching a stale terminal of an earlier one.
                let pty_id = existing.clone().unwrap_or_else(|| {
                    format!("tmux-{}-{}", run_id(), pane_id.trim_start_matches('%'))
                });
                pane.pty_id = Some(pty_id.clone());
                let title = if pane.title.is_empty() {
                    format!("teammate {pane_id}")
                } else {
                    pane.title.clone()
                };
                let leader_pty = pane.leader_pty.clone();
                state.pty_pane.insert(pty_id.clone(), pane_id.clone());
                (
                    Reply::ok(""),
                    vec![Effect::Open {
                        pane_id,
                        pty_id,
                        leader_pty,
                        title,
                        cwd,
                        command_line,
                        args,
                    }],
                )
            }
            "kill-pane" => {
                let pane_id = resolve_pane(&state, flag(rest, "-t"), &caller);
                let Some(pane) = state.panes.remove(&pane_id) else {
                    return (Reply::fail(format!("can't find pane: {pane_id}\n")), Vec::new());
                };
                if let Some(panes) = state.windows.get_mut(&pane.window) {
                    panes.retain(|id| id != &pane_id);
                }
                let mut effects = Vec::new();
                if let Some(pty_id) = pane.pty_id {
                    state.pty_pane.remove(&pty_id);
                    if pty_id != pane.leader_pty {
                        effects.push(Effect::Kill { pty_id });
                    }
                }
                (Reply::ok(""), effects)
            }
            "send-keys" => {
                let pane_id = resolve_pane(&state, flag(rest, "-t"), &caller);
                let Some(pty_id) = state.panes.get(&pane_id).and_then(|pane| pane.pty_id.clone())
                else {
                    return (Reply::fail(format!("can't find pane: {pane_id}\n")), Vec::new());
                };
                let data = keys_to_input(rest);
                let effects = if data.is_empty() {
                    Vec::new()
                } else {
                    vec![Effect::Write { pty_id, data }]
                };
                (Reply::ok(""), effects)
            }
            // Probes, styling, layout and the rest: accepted, nothing to do yet.
            _ => (Reply::ok(""), Vec::new()),
        }
    }
}

/// What `send-keys` types into a pane: key names become their bytes, anything else is typed as is.
/// With `-l` every argument is literal text.
fn keys_to_input(args: &[String]) -> String {
    let mut literal = false;
    let mut out = String::new();
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        index += 1;
        if arg == "-t" {
            index += 1;
            continue;
        }
        if arg == "-l" {
            literal = true;
            continue;
        }
        if arg.len() > 1 && arg.starts_with('-') && !literal {
            continue;
        }
        if literal {
            out.push_str(arg);
            continue;
        }
        match arg {
            "Enter" => out.push('\r'),
            "Space" => out.push(' '),
            "Tab" => out.push('\t'),
            "Escape" => out.push('\u{1b}'),
            "BSpace" => out.push('\u{7f}'),
            "Up" => out.push_str("\u{1b}[A"),
            "Down" => out.push_str("\u{1b}[B"),
            "Right" => out.push_str("\u{1b}[C"),
            "Left" => out.push_str("\u{1b}[D"),
            _ => match arg.strip_prefix("C-").map(|rest| rest.chars().collect::<Vec<_>>()) {
                Some(chars) if chars.len() == 1 && chars[0].is_ascii_alphabetic() => {
                    out.push(((chars[0].to_ascii_lowercase() as u8) & 0x1f) as char);
                }
                _ => out.push_str(arg),
            },
        }
    }
    out
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn resolve_pane(state: &State, target: Option<&str>, caller: &str) -> String {
    match target {
        // An explicit pane that does not exist stays as it is, so the command fails instead of
        // acting on the caller's own pane.
        Some(target) if target.starts_with('%') => target.to_string(),
        // A window target, or a session/window path such as `claude:0.0`: its first pane.
        Some(target) if target.starts_with('@') => state
            .windows
            .get(target)
            .and_then(|panes| panes.first().cloned())
            .unwrap_or_else(|| caller.to_string()),
        _ => caller.to_string(),
    }
}

fn resolve_window(state: &State, target: Option<&str>, caller: &str) -> String {
    match target {
        Some(target) if target.starts_with('@') && state.windows.contains_key(target) => {
            target.to_string()
        }
        _ => {
            let pane = resolve_pane(state, target, caller);
            state
                .panes
                .get(&pane)
                .map(|pane| pane.window.clone())
                .unwrap_or_else(|| "@0".to_string())
        }
    }
}

fn expand(state: &State, format: &str, pane_id: &str) -> String {
    let window = state
        .panes
        .get(pane_id)
        .map(|pane| pane.window.clone())
        .unwrap_or_else(|| "@0".to_string());
    let title = state
        .panes
        .get(pane_id)
        .map(|pane| pane.title.clone())
        .unwrap_or_default();
    let mut out = String::new();
    let mut rest = format;
    while let Some(start) = rest.find("#{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        out.push_str(&match &after[..end] {
            "pane_id" => pane_id.to_string(),
            "window_id" => window.clone(),
            "window_index" | "pane_index" => "0".to_string(),
            "session_name" => "alethe".to_string(),
            "session_id" => "$0".to_string(),
            "pane_title" => title.clone(),
            "client_termtype" => "xterm-256color".to_string(),
            _ => String::new(),
        });
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Splits a POSIX-ish command line into words, honouring single and double quotes.
fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                started = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    current.push(q);
                }
            }
            '"' => {
                started = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' if matches!(chars.peek(), Some('"' | '\\')) => {
                            current.push(chars.next().expect("peeked"));
                        }
                        _ => current.push(q),
                    }
                }
            }
            c if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                started = true;
                current.push(c);
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}

fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Splits `a && b && c` outside quotes.
fn split_and(line: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => {
                quote = Some(c);
                current.push(c);
            }
            (Some(q), c) if c == q => {
                quote = None;
                current.push(c);
            }
            (None, '&') if chars.peek() == Some(&'&') => {
                chars.next();
                segments.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    segments.push(current);
    segments
}

/// The directory a command line changes into first, and the line to run in the PTY's shell. On
/// Windows that shell is PowerShell, so Claude Code's `cd 'dir' && env K=V 'exe' args` is rewritten.
fn translate_command(line: &str) -> (Option<String>, String) {
    let mut cwd = None;
    let mut parts: Vec<String> = Vec::new();
    for segment in split_and(line) {
        let words = split_words(&segment);
        match words.first().map(String::as_str) {
            Some("cd") if words.len() == 2 => {
                if cwd.is_none() {
                    cwd = Some(words[1].clone());
                }
                parts.push(format!(
                    "Set-Location -LiteralPath {} -ErrorAction Stop",
                    ps_quote(&words[1])
                ));
            }
            Some(_) => {
                let mut index = 0;
                if words[0] == "env" {
                    index = 1;
                }
                while index < words.len() {
                    let Some((key, value)) = words[index].split_once('=') else {
                        break;
                    };
                    if key.is_empty()
                        || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        break;
                    }
                    parts.push(format!("$env:{key} = {}", ps_quote(value)));
                    index += 1;
                }
                if index < words.len() {
                    let args: Vec<String> = words[index + 1..].iter().map(|w| ps_quote(w)).collect();
                    parts.push(format!("& {} {}", ps_quote(&words[index]), args.join(" ")));
                }
            }
            None => {}
        }
    }
    if cfg!(windows) {
        (cwd, parts.join("; "))
    } else {
        (cwd, line.to_string())
    }
}

/// `alethe-team open <role> [task…]` or `alethe-team send <role> <message…>`.
fn team_command(args: &[String], leader_pty: Option<&str>) -> (Reply, Vec<Effect>) {
    let usage = "usage: alethe-team open <role> [task] | alethe-team send <role> <message>\n";
    let (Some(action @ ("open" | "send")), Some(role)) =
        (args.first().map(String::as_str), args.get(1))
    else {
        return (Reply::fail(usage), Vec::new());
    };
    let valid_role = !role.is_empty()
        && role.len() <= 24
        && role.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid_role {
        return (
            Reply::fail("alethe-team: the role is one word of letters, digits, - or _\n"),
            Vec::new(),
        );
    }
    let Some(leader_pty) = leader_pty.filter(|pty| !pty.is_empty()) else {
        return (
            Reply::fail("alethe-team: not running inside an Alethe terminal\n"),
            Vec::new(),
        );
    };
    let text = args[2..].join(" ");
    if action == "open" {
        return (
            Reply::ok(format!("opening {role} next to this terminal\n")),
            vec![Effect::OpenMember {
                leader_pty: leader_pty.to_string(),
                role: role.clone(),
                task: text,
            }],
        );
    }
    let message = text.trim();
    if message.is_empty() {
        return (
            Reply::fail("alethe-team: send needs the task to give the member\n"),
            Vec::new(),
        );
    }
    if message.chars().count() > MAX_TEAM_MESSAGE {
        return (
            Reply::fail(format!(
                "alethe-team: the task is longer than {MAX_TEAM_MESSAGE} characters; put the details in TEAM.md\n"
            )),
            Vec::new(),
        );
    }
    (
        Reply::ok(format!("sent to {role}\n")),
        vec![Effect::SendMember {
            leader_pty: leader_pty.to_string(),
            role: role.clone(),
            message: message.to_string(),
        }],
    )
}

/// The arguments of the program a `cd … && env K=V 'exe' args` line runs.
fn executable_args(line: &str) -> Vec<String> {
    let Some(last) = split_and(line).pop() else {
        return Vec::new();
    };
    let words = split_words(&last);
    let mut index = usize::from(words.first().is_some_and(|word| word == "env"));
    while words.get(index).is_some_and(|word| {
        word.split_once('=').is_some_and(|(key, _)| {
            !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    }) {
        index += 1;
    }
    words.into_iter().skip(index + 1).collect()
}

/// The hooks settings file Claude Code was started with.
fn settings_path(args: &[String]) -> Option<&String> {
    let index = args.iter().position(|arg| arg == "--settings")?;
    args.get(index + 1)
}

/// Makes a teammate report its hooks as its own terminal. It inherits the leader's hooks file,
/// whose terminal id is written into it, and the leader would adopt the teammate's conversation
/// as its own. A file of the teammate's own carries its terminal id instead.
fn with_own_hooks(app: &AppHandle, pty_id: &str, args: &[String], command_line: String) -> String {
    let Some(old) = settings_path(args) else {
        return command_line;
    };
    let orchestrator = old.contains("-full-");
    match crate::agent_events::agent_hooks_settings_path(
        app.clone(),
        pty_id.to_string(),
        Some(orchestrator),
        None,
        None,
    ) {
        Ok(new) => command_line.replace(old.as_str(), &new),
        Err(_) => command_line,
    }
}

fn shim_dir() -> Option<PathBuf> {
    dirs_next::data_local_dir().map(|base| base.join("Alethe").join("tmux-shim"))
}

/// What the shim is made from: the small `alethe-tmux` binary when it sits next to the app (dev
/// builds, which start faster), otherwise the app itself, which acts as `tmux` when it is started
/// under that name (see `tmux_shim.rs`).
fn shim_source() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let small = exe
        .parent()?
        .join(if cfg!(windows) { "alethe-tmux.exe" } else { "alethe-tmux" });
    Some(if small.is_file() { small } else { exe })
}

/// Whether `dest` is already a current copy of `source`.
fn shim_is_current(source: &std::path::Path, dest: &std::path::Path) -> bool {
    let (Ok(source), Ok(dest)) = (std::fs::metadata(source), std::fs::metadata(dest)) else {
        return false;
    };
    source.len() == dest.len()
        && matches!((source.modified(), dest.modified()), (Ok(s), Ok(d)) if d >= s)
}

/// Puts the shim, named `tmux`, where a terminal's PATH can find it. A hard link when the volume
/// allows it, so a big dev binary is not copied; otherwise a copy. A shim that cannot be replaced
/// because a terminal is running it is kept as it is.
fn install_shim() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let source = shim_source()?;
        let dir = shim_dir()?;
        std::fs::create_dir_all(&dir).ok()?;
        // The same program under two names: it behaves as the one it is started as.
        for name in ["tmux", TEAM_COMMAND] {
            let dest = dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() });
            if shim_is_current(&source, &dest) {
                continue;
            }
            let _ = std::fs::remove_file(&dest);
            let installed =
                std::fs::hard_link(&source, &dest).is_ok() || std::fs::copy(&source, &dest).is_ok();
            if !installed && !dest.is_file() {
                eprintln!("[tmux_bridge] could not install {name} from {}", source.display());
                if name == "tmux" {
                    return None;
                }
            }
        }
        Some(dir)
    })
    .clone()
}

/// Whether a terminal started with `agent` gets the tmux look-alike: Claude Code, and plain
/// shells (teammates and a `claude` typed by hand). Every other agent is left alone, because a
/// `TMUX` variable changes how some of them draw.
fn applies_to(agent: Option<&str>) -> bool {
    let Some(agent) = agent.map(str::trim).filter(|agent| !agent.is_empty()) else {
        return true;
    };
    is_claude(agent)
}

fn is_claude(agent: &str) -> bool {
    let name = agent.rsplit(['/', '\\']).next().unwrap_or(agent).to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name) == "claude"
}

fn is_antigravity(agent: &str) -> bool {
    let name = agent.rsplit(['/', '\\']).next().unwrap_or(agent).to_ascii_lowercase();
    matches!(name.strip_suffix(".exe").unwrap_or(&name), "agy" | "antigravity")
}

/// Whether a new terminal should start without the scrollback saved by its previous run. Claude
/// redraws its whole screen on resume, and replaying the old frames under it (drawn at another
/// width) leaves a garbled screen.
pub fn discards_saved_scrollback(agent: Option<&str>) -> bool {
    enabled() && agent.map(str::trim).is_some_and(is_claude)
}

/// Environment that makes a terminal look like it runs inside tmux, so Claude Code opens its
/// teammates as panes. Nothing is added unless the bridge is on and the shim is installed.
pub fn apply_env(command: &mut portable_pty::CommandBuilder, pty_id: &str, agent: Option<&str>) {
    let tmux_like = applies_to(agent);
    let antigravity = agent.map(str::trim).is_some_and(is_antigravity);
    if !enabled() || !(tmux_like || antigravity) {
        return;
    }
    let Some(dir) = install_shim() else { return };
    let Some((port, token)) = crate::agent_events::listener_info() else {
        return;
    };
    let key = if cfg!(windows) { "Path" } else { "PATH" };
    let separator = if cfg!(windows) { ';' } else { ':' };
    let existing = command
        .get_env(key)
        .map(|value| value.to_string_lossy().to_string())
        .or_else(|| std::env::var(key).ok())
        .unwrap_or_default();
    command.env(key, format!("{}{separator}{existing}", dir.display()));
    command.env("ALETHE_TMUX_PORT", port.to_string());
    command.env("ALETHE_TMUX_TOKEN", token);
    if antigravity {
        // Only the `alethe-team` helper: a `TMUX` variable changes how some agents draw.
        command.env("ALETHE_TEAM_PTY", pty_id);
        return;
    }
    let (pane_id, _window) = bridge().register_leader(pty_id);
    command.env("TMUX", format!("{},1,0", dir.join("alethe.sock").display()));
    command.env("TMUX_PANE", pane_id);
    command.env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");
    // Inside tmux, Claude Code drops to 256 colours (its orange turns pink). The terminal here
    // does draw truecolor, so say so.
    command.env("CLAUDE_CODE_TMUX_TRUECOLOR", "1");
}

/// Answers `alethe-team` calls that arrive as files (see `tmux_shim::spool_dir`). Only team
/// commands carrying the listener token are served; every file is removed once read.
pub fn watch_spool(app: AppHandle, port: u16) {
    let dir = crate::tmux_shim::spool_dir(&port.to_string());
    let _ = std::fs::create_dir_all(&dir);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("req") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let _ = std::fs::remove_file(&path);
            let reply = serve_spooled(&app, &text);
            let part = path.with_extension("rpart");
            if std::fs::write(&part, serde_json::to_string(&reply).unwrap_or_default()).is_ok() {
                let _ = std::fs::rename(&part, path.with_extension("rep"));
            }
        }
    });
}

fn serve_spooled(app: &AppHandle, text: &str) -> Reply {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Reply::fail("alethe-team: unreadable request\n");
    };
    let token = value["token"].as_str().unwrap_or("");
    if !crate::agent_events::token_matches(token) {
        return Reply::fail("alethe-team: request refused\n");
    }
    match serde_json::from_value::<Request>(value) {
        Ok(request) if request.args.first().map(String::as_str) == Some(TEAM_COMMAND) => {
            handle(app, &request)
        }
        _ => Reply::fail("alethe-team: only team commands are accepted here\n"),
    }
}

/// Runs one shim request: updates the registry, then performs what the command asked for.
pub fn handle(app: &AppHandle, request: &Request) -> Reply {
    let (mut reply, effects) = bridge().plan(request);
    for effect in effects {
        match effect {
            Effect::Open {
                pane_id,
                pty_id,
                leader_pty,
                title,
                cwd,
                command_line,
                args,
            } => {
                let command_line = with_own_hooks(app, &pty_id, &args, command_line);
                let spawned = tauri::async_runtime::block_on(pty::spawn_pty(
                    app.clone(),
                    app.state::<PtySessions>(),
                    app.state::<std::sync::Arc<crate::remote::RemoteHub>>(),
                    HEADLESS_COLS,
                    HEADLESS_ROWS,
                    Some(pty_id.clone()),
                    None,
                    cwd.clone(),
                    None,
                    None,
                    None,
                    None,
                    Some(command_line),
                ));
                match spawned {
                    Ok(_) => {
                        let _ = app.emit(
                            OPEN_EVENT,
                            json!({
                                "paneId": pane_id,
                                "ptyId": pty_id,
                                "leaderPtyId": leader_pty,
                                "title": title,
                                "cwd": cwd,
                                "args": args,
                            }),
                        );
                    }
                    Err(error) => reply = Reply::fail(format!("respawn-pane: {error}\n")),
                }
            }
            Effect::OpenMember {
                leader_pty,
                role,
                task,
            } => {
                let _ = app.emit(
                    OPEN_MEMBER_EVENT,
                    json!({ "leaderPtyId": leader_pty, "role": role, "task": task }),
                );
            }
            Effect::SendMember {
                leader_pty,
                role,
                message,
            } => {
                let _ = app.emit(
                    SEND_MEMBER_EVENT,
                    json!({ "leaderPtyId": leader_pty, "role": role, "message": message }),
                );
            }
            Effect::Write { pty_id, data } => {
                if let Err(error) = tauri::async_runtime::block_on(pty::write_pty(
                    app.state::<PtySessions>(),
                    pty_id,
                    data,
                )) {
                    reply = Reply::fail(format!("send-keys: {error}\n"));
                }
            }
            Effect::Kill { pty_id } => {
                let _ = tauri::async_runtime::block_on(pty::kill_pty(
                    app.clone(),
                    app.state::<PtySessions>(),
                    pty_id.clone(),
                ));
                let _ = app.emit(CLOSE_EVENT, json!({ "ptyId": pty_id }));
            }
        }
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bridge: &Bridge, pane: &str, args: &[&str]) -> (Reply, Vec<Effect>) {
        bridge.plan(&Request {
            args: args.iter().map(|arg| arg.to_string()).collect(),
            pane: Some(pane.to_string()),
        })
    }

    #[test]
    fn a_shim_is_current_only_when_it_matches_the_source() {
        let dir = std::env::temp_dir().join(format!("alethe-shim-test-{}", run_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source");
        let dest = dir.join("dest");
        std::fs::write(&source, b"one").unwrap();
        assert!(!shim_is_current(&source, &dest), "a missing shim is not current");
        std::fs::copy(&source, &dest).unwrap();
        assert!(shim_is_current(&source, &dest));
        std::fs::write(&source, b"longer").unwrap();
        assert!(!shim_is_current(&source, &dest), "a different size is not current");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_claude_and_plain_shells_get_the_tmux_look_alike() {
        assert!(applies_to(None));
        assert!(applies_to(Some("")));
        assert!(applies_to(Some("claude")));
        assert!(applies_to(Some(r"C:\Users\me\.local\bin\claude.exe")));
        assert!(!applies_to(Some("codex")));
        assert!(!applies_to(Some("opencode")));
        assert!(!applies_to(Some("agy")));
        assert!(!applies_to(Some("cursor-agent")));
    }

    #[test]
    fn finds_the_hooks_file_a_teammate_inherited() {
        let args: Vec<String> = ["--agent-id", "a@t", "--settings", "C:\\h-full-x.json", "--x"]
            .iter()
            .map(|arg| arg.to_string())
            .collect();
        assert_eq!(settings_path(&args).map(String::as_str), Some("C:\\h-full-x.json"));
        assert_eq!(settings_path(&args[..2]), None);
    }

    #[test]
    fn a_lead_opens_a_team_member_with_a_role_and_a_task() {
        let bridge = Bridge::default();
        let (reply, effects) = run(&bridge, "pty-lead", &["alethe-team", "open", "front", "fix", "the", "menu"]);
        assert_eq!(reply.code, 0);
        assert_eq!(
            effects,
            vec![Effect::OpenMember {
                leader_pty: "pty-lead".into(),
                role: "front".into(),
                task: "fix the menu".into(),
            }]
        );
    }

    #[test]
    fn a_lead_sends_a_new_task_to_a_member_that_is_already_open() {
        let bridge = Bridge::default();
        let (reply, effects) = run(
            &bridge,
            "pty-lead",
            &["alethe-team", "send", "back", "add", "the", "login", "route"],
        );
        assert_eq!(reply.code, 0);
        assert_eq!(
            effects,
            vec![Effect::SendMember {
                leader_pty: "pty-lead".into(),
                role: "back".into(),
                message: "add the login route".into(),
            }]
        );
    }

    #[test]
    fn sending_needs_a_task_a_valid_role_and_a_known_caller() {
        let bridge = Bridge::default();
        let (reply, effects) = run(&bridge, "pty-lead", &["alethe-team", "send", "back"]);
        assert_eq!(reply.code, 1);
        assert!(effects.is_empty());
        let (reply, effects) = run(&bridge, "pty-lead", &["alethe-team", "send", "bad role!", "x"]);
        assert_eq!(reply.code, 1);
        assert!(effects.is_empty());
        let long = "x".repeat(MAX_TEAM_MESSAGE + 1);
        let (reply, effects) = run(
            &bridge,
            "pty-lead",
            &["alethe-team", "send", "back", long.as_str()],
        );
        assert_eq!(reply.code, 1);
        assert!(effects.is_empty());
        let outside = bridge.plan(&Request {
            args: vec!["alethe-team".into(), "send".into(), "back".into(), "hi".into()],
            pane: None,
        });
        assert_eq!(outside.0.code, 1);
    }

    #[test]
    fn the_team_helper_rejects_a_bad_role_or_an_unknown_caller() {
        let bridge = Bridge::default();
        let (reply, effects) = run(&bridge, "pty-lead", &["alethe-team", "open", "bad role!"]);
        assert_eq!(reply.code, 1);
        assert!(effects.is_empty());
        let (reply, _) = run(&bridge, "pty-lead", &["alethe-team", "close", "front"]);
        assert_eq!(reply.code, 1);
        let outside = bridge.plan(&Request {
            args: vec!["alethe-team".into(), "open".into(), "front".into()],
            pane: None,
        });
        assert_eq!(outside.0.code, 1);
    }

    #[test]
    fn antigravity_is_recognised_by_its_command_name() {
        assert!(is_antigravity("agy"));
        assert!(is_antigravity(r"C:\Users\me\AppData\Local\agy\bin\agy.exe"));
        assert!(is_antigravity("antigravity"));
        assert!(!is_antigravity("claude"));
        assert!(!is_antigravity("codex"));
    }

    #[test]
    fn saved_scrollback_is_discarded_only_for_claude() {
        ENABLED.store(true, Ordering::Relaxed);
        assert!(discards_saved_scrollback(Some("claude")));
        assert!(discards_saved_scrollback(Some(r"C:\bin\claude.exe")));
        assert!(!discards_saved_scrollback(None));
        assert!(!discards_saved_scrollback(Some("codex")));
        ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn reports_a_version_so_claude_finds_tmux() {
        let bridge = Bridge::default();
        assert_eq!(run(&bridge, "%0", &["-V"]).0.stdout, "tmux 3.4\n");
    }

    #[test]
    fn a_leader_and_its_teammates_share_a_window() {
        let bridge = Bridge::default();
        let (leader, window) = bridge.register_leader("pty-a");
        assert_eq!((leader.as_str(), window.as_str()), ("%0", "@0"));
        assert_eq!(bridge.register_leader("pty-a").0, "%0");

        let split = run(
            &bridge,
            "%0",
            &["split-window", "-d", "-t", "%0", "-h", "-P", "-F", "#{pane_id}", "--", "cat"],
        );
        assert_eq!(split.0.stdout, "%1\n");
        let listed = run(&bridge, "%0", &["-S", "sock", "list-panes", "-t", "@0", "-F", "#{pane_id}"]);
        assert_eq!(listed.0.stdout, "%0\n%1\n");
    }

    #[test]
    fn respawn_opens_a_pane_with_the_title_set_before_it() {
        let bridge = Bridge::default();
        bridge.register_leader("pty-a");
        run(&bridge, "%0", &["split-window", "-t", "%0", "-P"]);
        run(&bridge, "%0", &["select-pane", "-t", "%1", "-T", "security-reviewer"]);
        let (reply, effects) = run(
            &bridge,
            "%0",
            &["respawn-pane", "-k", "-t", "%1", "--", "cd '/work' && env A=1 'claude' --x"],
        );
        assert_eq!(reply.code, 0);
        match &effects[..] {
            [Effect::Open { pane_id, pty_id, leader_pty, title, cwd, command_line, args }] => {
                assert_eq!(args, &["--x".to_string()]);
                assert_eq!(pane_id, "%1");
                assert_eq!(pty_id, &format!("tmux-{}-1", run_id()));
                assert!(command_line.contains("claude"));
                assert_eq!(leader_pty, "pty-a");
                assert_eq!(title, "security-reviewer");
                assert_eq!(cwd.as_deref(), Some("/work"));
            }
            other => panic!("unexpected effects: {other:?}"),
        }
    }

    #[test]
    fn killing_a_teammate_pane_kills_its_pty_but_never_the_leader() {
        let bridge = Bridge::default();
        bridge.register_leader("pty-a");
        run(&bridge, "%0", &["split-window", "-t", "%0", "-P"]);
        run(&bridge, "%0", &["respawn-pane", "-t", "%1", "--", "claude"]);
        let (_, effects) = run(&bridge, "%0", &["kill-pane", "-t", "%1"]);
        assert_eq!(
            effects,
            vec![Effect::Kill {
                pty_id: format!("tmux-{}-1", run_id())
            }]
        );
        let (_, effects) = run(&bridge, "%0", &["kill-pane", "-t", "%0"]);
        assert!(effects.is_empty());
    }

    #[test]
    fn display_message_expands_known_formats_and_blanks_the_rest() {
        let bridge = Bridge::default();
        bridge.register_leader("pty-a");
        let reply = run(&bridge, "%0", &["display-message", "-p", "-t", "%0", "#{pane_id}|#{window_id}|#{nope}"]);
        assert_eq!(reply.0.stdout, "%0|@0|\n");
    }

    #[test]
    fn send_keys_types_into_the_teammates_pty() {
        let bridge = Bridge::default();
        bridge.register_leader("pty-a");
        run(&bridge, "%0", &["split-window", "-t", "%0", "-P"]);
        run(&bridge, "%0", &["respawn-pane", "-t", "%1", "--", "claude"]);
        let (reply, effects) = run(&bridge, "%0", &["send-keys", "-t", "%1", "hello", "Space", "C-c", "Enter"]);
        assert_eq!(reply.code, 0);
        assert_eq!(
            effects,
            vec![Effect::Write {
                pty_id: format!("tmux-{}-1", run_id()),
                data: "hello \u{3}\r".into()
            }]
        );
        let (reply, _) = run(&bridge, "%0", &["send-keys", "-t", "%9", "x"]);
        assert_eq!(reply.code, 1);
    }

    #[test]
    fn killing_an_unknown_pane_leaves_the_callers_pane_alone() {
        let bridge = Bridge::default();
        bridge.register_leader("pty-a");
        let (reply, effects) = run(&bridge, "%0", &["kill-pane", "-t", "%9"]);
        assert_eq!(reply.code, 1);
        assert!(effects.is_empty());
        assert_eq!(run(&bridge, "%0", &["list-panes", "-t", "@0"]).0.stdout, "%0\n");
    }

    #[test]
    fn literal_send_keys_does_not_translate_key_names() {
        let args: Vec<String> = ["-t", "%1", "-l", "Enter"].iter().map(|s| s.to_string()).collect();
        assert_eq!(keys_to_input(&args), "Enter");
    }

    #[test]
    fn splits_quoted_words() {
        assert_eq!(
            split_words(r#"env A='b c' "d e" f"#),
            vec!["env", "A=b c", "d e", "f"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn rewrites_claude_codes_bash_line_for_powershell() {
        let (cwd, line) = translate_command(
            r"cd 'C:\Users\me\proj' && env CLAUDECODE=1 EMPTY= 'C:\bin\claude.exe' --agent-id a@b --name x",
        );
        assert_eq!(cwd.as_deref(), Some(r"C:\Users\me\proj"));
        assert_eq!(
            line,
            r"Set-Location -LiteralPath 'C:\Users\me\proj' -ErrorAction Stop; $env:CLAUDECODE = '1'; $env:EMPTY = ''; & 'C:\bin\claude.exe' '--agent-id' 'a@b' '--name' 'x'"
        );
    }
}
