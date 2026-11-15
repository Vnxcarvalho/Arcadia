//! `tmux` stand-in for terminals opened by Alethe.
//!
//! Claude Code's agent teams open a pane per teammate through the `tmux` CLI. Inside an Alethe
//! terminal `tmux` resolves to this code, which forwards every call to the app's local listener;
//! the listener answers it and opens the matching Alethe terminal. See `tmux_bridge.rs`.
//!
//! Compiled into the app, which behaves as the shim when it is started under the name `tmux`, and
//! into the small `alethe-tmux` binary, which dev builds prefer because it starts faster. Keep this
//! file free of Tauri so both can include it.
//!
//! Environment (set by the app): ALETHE_TMUX_PORT, ALETHE_TMUX_TOKEN, TMUX_PANE.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{json, Value};

/// The name an Antigravity lead runs to open a team member (`tmux_bridge.rs` handles it).
const TEAM_NAME: &str = "alethe-team";

fn is_team_path(path: &std::path::Path) -> bool {
    path.file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case(TEAM_NAME))
}

fn is_tmux_path(path: &std::path::Path) -> bool {
    path.file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("tmux") || stem.eq_ignore_ascii_case(TEAM_NAME))
}

/// Whether this process was started as `tmux` or `alethe-team` (with or without `.exe`).
pub fn invoked_as_tmux() -> bool {
    std::env::current_exe().is_ok_and(|path| is_tmux_path(&path))
}

#[cfg(test)]
mod tests {
    use super::is_tmux_path;
    use std::path::Path;

    #[test]
    fn recognises_the_tmux_name_and_nothing_else() {
        // Backslash separators only split into components on Windows.
        #[cfg(windows)]
        {
            assert!(is_tmux_path(Path::new(r"C:\Users\me\AppData\Local\Alethe\tmux-shim\tmux.exe")));
            assert!(!is_tmux_path(Path::new(r"C:\Program Files\Alethe\alethe.exe")));
        }
        assert!(is_tmux_path(Path::new("/home/me/.local/share/alethe/tmux")));
        assert!(is_tmux_path(Path::new("TMUX.EXE")));
        assert!(!is_tmux_path(Path::new("alethe-tmux.exe")));
        assert!(!is_tmux_path(Path::new("tmux-shim.exe")));
    }

    #[test]
    fn the_team_helper_is_a_shim_too_but_not_tmux() {
        #[cfg(windows)]
        assert!(is_tmux_path(Path::new(r"C:\x\tmux-shim\alethe-team.exe")));
        assert!(is_tmux_path(Path::new("alethe-team")));
        assert!(super::is_team_path(Path::new("alethe-team.exe")));
        assert!(!super::is_team_path(Path::new("tmux.exe")));
    }
}

/// Where the shim leaves requests when it cannot reach the app's listener. Antigravity runs its
/// commands in a sandbox that blocks loopback TCP and every folder but the temp one, so the temp
/// folder is the one place both sides can use.
pub fn spool_dir(port: &str) -> std::path::PathBuf {
    std::env::temp_dir().join("arcadia-team").join(port)
}

/// Hands the request to the app through `spool_dir` and waits for its answer.
fn spool_call(port: &str, body: &str, token: &str) -> Result<String, String> {
    let dir = spool_dir(port);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let mut request: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    request["token"] = json!(token);
    let pending = dir.join(format!("{nonce}.part"));
    std::fs::write(&pending, request.to_string()).map_err(|error| error.to_string())?;
    std::fs::rename(&pending, dir.join(format!("{nonce}.req"))).map_err(|error| error.to_string())?;
    let answer = dir.join(format!("{nonce}.rep"));
    for _ in 0..300 {
        if let Ok(text) = std::fs::read_to_string(&answer) {
            let _ = std::fs::remove_file(&answer);
            return Ok(text);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_file(dir.join(format!("{nonce}.req")));
    Err("Alethe did not answer".to_string())
}

/// Forwards this process's arguments to the app and exits with the app's answer.
pub fn run() -> ! {
    let team = std::env::current_exe().is_ok_and(|path| is_team_path(&path));
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if team {
        args.insert(0, TEAM_NAME.to_string());
    }
    let (Ok(port), Ok(token)) = (
        std::env::var("ALETHE_TMUX_PORT"),
        std::env::var("ALETHE_TMUX_TOKEN"),
    ) else {
        fail("alethe-tmux: not running inside an Alethe terminal");
    };
    // tmux names the caller by pane; the team helper names it by the terminal's own PTY id.
    let pane = std::env::var(if team { "ALETHE_TEAM_PTY" } else { "TMUX_PANE" }).ok();
    let body = json!({ "args": args, "pane": pane }).to_string();

    let request = format!(
        "POST /tmux HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Alethe-Token: {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let direct = TcpStream::connect(format!("127.0.0.1:{port}")).and_then(|mut stream| {
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.write_all(request.as_bytes())?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw)?;
        Ok(String::from_utf8_lossy(&raw).into_owned())
    });
    let payload = match direct {
        Ok(response) => response
            .split_once("\r\n\r\n")
            .map(|(_, rest)| rest.to_string())
            .unwrap_or_default(),
        // A sandboxed Antigravity cannot open the socket: the team helper falls back to the spool.
        Err(_) if team => spool_call(&port, &body, &token)
            .unwrap_or_else(|error| fail(&format!("alethe-team: cannot reach Alethe: {error}"))),
        Err(error) => fail(&format!("alethe-tmux: cannot reach Alethe: {error}")),
    };
    let Ok(reply) = serde_json::from_str::<Value>(&payload) else {
        fail("alethe-tmux: unexpected reply from Alethe");
    };
    // A GUI-subsystem build can have no console to write to; a failed write must not abort.
    let _ = std::io::stdout().write_all(reply["stdout"].as_str().unwrap_or("").as_bytes());
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(reply["stderr"].as_str().unwrap_or("").as_bytes());
    std::process::exit(reply["code"].as_i64().unwrap_or(1) as i32);
}

fn fail(message: &str) -> ! {
    let _ = writeln!(std::io::stderr(), "{message}");
    std::process::exit(1);
}
