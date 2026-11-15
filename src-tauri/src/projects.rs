use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex as AsyncMutex;

use crate::paths::projects_file_path;
use crate::provider_common::now_ms;

static SAVE_MUTEX: OnceLock<AsyncMutex<()>> = OnceLock::new();

static LAST_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const STALE_WRITE_THRESHOLD_MS: i64 = 2000;

fn save_mutex() -> &'static AsyncMutex<()> {
    SAVE_MUTEX.get_or_init(|| AsyncMutex::new(()))
}

fn external_todo_path(content: &str) -> Option<PathBuf> {
    let parsed: Value = serde_json::from_str(content).ok()?;
    let directory = parsed
        .get("preferences")?
        .get("todoStoragePath")?
        .as_str()?
        .trim();
    if directory.is_empty() {
        return None;
    }
    Some(PathBuf::from(directory).join("todos.jsonc"))
}

fn merge_external_todos(content: String) -> String {
    let Some(path) = external_todo_path(&content) else {
        return content;
    };
    let Ok(external) = fs::read_to_string(path) else {
        return content;
    };
    let Ok(external_json) = serde_json::from_str::<Value>(&external) else {
        return content;
    };
    let Some(todos) = external_json.get("todos").filter(|value| value.is_array()) else {
        return content;
    };
    let Ok(mut parsed) = serde_json::from_str::<Value>(&content) else {
        return content;
    };
    parsed["todos"] = todos.clone();
    serde_json::to_string(&parsed).unwrap_or(content)
}

fn save_external_todos(content: &str) -> Result<(), String> {
    let Some(path) = external_todo_path(content) else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let parsed: Value = serde_json::from_str(content).map_err(|error| error.to_string())?;
    let external = serde_json::json!({
        "version": 1,
        "todos": parsed.get("todos").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
    });
    let temporary = path.with_extension("jsonc.tmp");
    fs::write(
        &temporary,
        serde_json::to_string_pretty(&external).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::rename(&temporary, &path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn load_projects(app: AppHandle) -> Result<Option<String>, String> {
    let path = projects_file_path(&app)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let profile_id = crate::profiles::ensure_profiles_index(&app)?.active_profile_id;
    let migrated = crate::spotify::migrate_client_secret_content(
        &content,
        &profile_id,
        &crate::secure_store::OsSecretStore,
    )?;
    if migrated != content {
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, &migrated).map_err(|error| error.to_string())?;
        fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    }
    Ok(Some(merge_external_todos(migrated)))
}

///

pub const PROJECTS_SAVED_EVENT: &str = "projects://saved";

#[tauri::command]
pub async fn save_projects(app: AppHandle, content: String, sequence: u64) -> Result<(), String> {
    let _guard = save_mutex().lock().await;

    let rust_now = now_ms();
    let last = LAST_WRITE_SEQUENCE.load(Ordering::SeqCst);

    if sequence <= last {
        let delay_ms = rust_now as i64 - sequence as i64;
        if delay_ms > STALE_WRITE_THRESHOLD_MS {
            return Ok(());
        }

        eprintln!(
            "[projects] aviso: sequence {sequence} <= last {last}, mas dentro do limiar \
             de {STALE_WRITE_THRESHOLD_MS}ms (possível recuo de relógio) — gravando mesmo assim."
        );
    }

    let path = projects_file_path(&app)?;
    let profile_id = crate::profiles::ensure_profiles_index(&app)?.active_profile_id;
    let content = crate::spotify::migrate_client_secret_content(
        &content,
        &profile_id,
        &crate::secure_store::OsSecretStore,
    )?;
    // I/O em spawn_blocking: escrever/renomear em disco lento (rede, AV scan)

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &content).map_err(|error| error.to_string())?;
        fs::rename(&tmp, &path).map_err(|error| error.to_string())?;
        save_external_todos(&content)?;
        Ok(())
    })
    .await
    .map_err(|error| format!("save_projects: falha na task bloqueante: {error}"))??;

    LAST_WRITE_SEQUENCE.store(sequence, Ordering::SeqCst);
    // A detached orchestration board only reads projects.json; this is its cue to read it again.
    let _ = app.emit(PROJECTS_SAVED_EVENT, ());
    Ok(())
}

fn repo_folder_name(normalized_url: &str) -> String {
    let name = normalized_url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("repo")
        .trim_end_matches(".git");
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "repo".to_string()
    } else {
        sanitized
    }
}

/// Where clones that Arcadia manages live. The user never picks these folders; they are removed
/// when the project is deleted (see `remove_managed_clone`).
fn managed_clones_root() -> Option<std::path::PathBuf> {
    dirs_next::data_local_dir().map(|base| base.join("Alethe").join("clones"))
}

fn resolve_clone_target(requested: &str, normalized_url: &str) -> Result<String, String> {
    let folder = repo_folder_name(normalized_url);
    let trimmed = requested.trim();
    let base = if trimmed.is_empty() {
        managed_clones_root().ok_or_else(|| "clone_git_missing:no local data folder".to_string())?
    } else {
        std::path::PathBuf::from(trimmed)
    };
    let target = if base.file_name().and_then(|n| n.to_str()) == Some(folder.as_str()) {
        base
    } else {
        base.join(&folder)
    };
    Ok(target.to_string_lossy().to_string())
}

fn normalize_github_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("git@")
    {
        trimmed.to_string()
    } else if trimmed.starts_with("github.com/") {
        format!("https://{trimmed}")
    } else if trimmed.contains('/') && !trimmed.contains(' ') {
        format!("https://github.com/{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// Where a clone ended up, and whether the folder already held that repository.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneOutcome {
    pub path: String,
    pub already_cloned: bool,
}

/// `host/owner/repo` in lowercase, so two spellings of one repository address compare equal
/// (`https://github.com/a/b.git`, `git@github.com:a/b`, a URL with credentials, a trailing slash).
fn repo_identity(url: &str) -> String {
    let mut value = url.trim().to_lowercase();
    for prefix in ["https://", "http://", "ssh://", "git://", "git@"] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest.to_string();
            break;
        }
    }
    let value = value.replace(':', "/");
    let value = match value.split_once('@') {
        Some((_, rest)) => rest.to_string(),
        None => value,
    };
    value
        .trim_start_matches("www.")
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string()
}

#[derive(Debug, PartialEq, Eq)]
enum CloneTarget {
    /// Nothing there, or an empty folder: `git clone` can use it.
    Free,
    /// The folder already is a clone of the wanted repository.
    AlreadyCloned,
    /// Something else lives there, so cloning would fail or mix two projects.
    InUse,
}

fn clone_target_state(target: &std::path::Path, wanted_url: &str) -> CloneTarget {
    if !target.exists() {
        return CloneTarget::Free;
    }
    if target.join(".git").exists() {
        let origin = crate::git_control::git_command(target, &["remote", "get-url", "origin"])
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
        if origin
            .as_deref()
            .is_some_and(|origin| repo_identity(origin) == repo_identity(wanted_url))
        {
            return CloneTarget::AlreadyCloned;
        }
    }
    let empty = fs::read_dir(target).is_ok_and(|mut entries| entries.next().is_none());
    if empty {
        CloneTarget::Free
    } else {
        CloneTarget::InUse
    }
}

/// Deletes a managed clone once nothing in it would be lost. Returns `removed`, or `not_managed`
/// for any folder outside the managed clones root (those are the user's and are never touched).
/// Fails with `clone_has_changes` when it holds uncommitted work or commits that were never pushed.
#[tauri::command]
pub async fn remove_managed_clone(path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let Some(root) = managed_clones_root().and_then(|root| root.canonicalize().ok()) else {
            return Ok("not_managed".to_string());
        };
        let Ok(target) = std::path::PathBuf::from(&path).canonicalize() else {
            return Ok("not_managed".to_string());
        };
        if target == root || !target.starts_with(&root) {
            return Ok("not_managed".to_string());
        }
        let git = |args: &[&str]| crate::git_control::git_command(&target, args);
        let dirty = git(&["status", "--porcelain"])?;
        let unpushed = git(&["rev-list", "--count", "HEAD", "--not", "--remotes"])?;
        let clean = dirty.status.success() && String::from_utf8_lossy(&dirty.stdout).trim().is_empty();
        let pushed = unpushed.status.success()
            && String::from_utf8_lossy(&unpushed.stdout).trim() == "0";
        // The files Arcadia writes into a clone (briefing files, opencode.json) are not work.
        let only_arcadia_files = !clean
            && String::from_utf8_lossy(&dirty.stdout).lines().all(|line| {
                let name = line.get(3..).unwrap_or("").trim();
                line.starts_with("??") && matches!(name, "AGENTS.md" | "CLAUDE.md" | "opencode.json")
            });
        if !(pushed && (clean || only_arcadia_files)) {
            return Err("clone_has_changes".to_string());
        }
        fs::remove_dir_all(&target).map_err(|error| format!("clone_remove_failed:{error}"))?;
        Ok("removed".to_string())
    })
    .await
    .map_err(|error| format!("clone_remove_failed:{error}"))?
}

/// Clones a repository. Errors are short codes the interface turns into a sentence in the user's
/// language: `clone_target_not_empty:<path>`, `clone_failed:<git's message>`,
/// `clone_git_missing:<reason>`.
#[tauri::command]
pub async fn clone_github_repo(
    app: tauri::AppHandle,
    url: String,
    target_dir: String,
) -> Result<CloneOutcome, String> {
    tokio::task::spawn_blocking(move || {
        let normalized = normalize_github_url(&url);
        let requested_dir = target_dir.clone();
        let target_dir = resolve_clone_target(&target_dir, &normalized)?;
        let target_path = std::path::PathBuf::from(&target_dir);

        // No folder chosen: a managed clone, shallow to keep the disk use small.
        let managed = requested_dir.trim().is_empty();
        match clone_target_state(&target_path, &normalized) {
            CloneTarget::AlreadyCloned => {
                return Ok(CloneOutcome {
                    path: target_dir,
                    already_cloned: true,
                });
            }
            CloneTarget::InUse => return Err(format!("clone_target_not_empty:{target_dir}")),
            CloneTarget::Free => {}
        }

        if let Some(parent) = target_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let mut clone_args = vec!["clone"];
        if managed {
            // A managed clone is shallow: only the current files, so it takes little disk space.
            clone_args.extend(["--depth", "1"]);
        }
        clone_args.extend([normalized.as_str(), target_dir.as_str()]);
        let mut cmd = crate::git_control::git_process(&target_dir, &clone_args);
        crate::git_control::hide_console(&mut cmd);

        // Signed in to GitHub: authenticate https clones so private repositories work.
        let token = if normalized.starts_with("https://github.com/") {
            crate::github_account::token_for_clone(&app)
        } else {
            None
        };
        if let Some(token) = token.as_deref() {
            cmd.envs(crate::github_account::git_auth_env(token));
        }

        let output = cmd
            .output()
            .map_err(|e| format!("clone_git_missing:{e}"))?;

        if !output.status.success() {
            let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if let Some(token) = token.as_deref() {
                stderr = stderr.replace(token, "***");
            }
            return Err(format!("clone_failed:{}", stderr.trim()));
        }

        // Gera os arquivos de briefing de contexto para os agentes de IA
        let _ = generate_repo_context_files(&target_dir);

        let _ = crate::graphify::graphify_opencode_config_write_inner(target_dir.clone(), None);

        Ok(CloneOutcome {
            path: target_dir,
            already_cloned: false,
        })
    })
    .await
    .map_err(|e| format!("clone_failed:{e}"))?
}

fn generate_repo_context_files(project_dir: &str) -> Result<(), String> {
    let path = std::path::PathBuf::from(project_dir);
    if !path.exists() {
        return Ok(());
    }

    let readme_content = ["README.md", "README.txt", "readme.md", "README"]
        .iter()
        .find_map(|f| fs::read_to_string(path.join(f)).ok())
        .unwrap_or_else(|| "Nenhum README encontrado.".to_string());

    let mut tech_stack = Vec::new();
    if path.join("package.json").exists() {
        tech_stack.push("Node.js / JavaScript / TypeScript");
    }
    if path.join("Cargo.toml").exists() {
        tech_stack.push("Rust");
    }
    if path.join("pyproject.toml").exists() || path.join("requirements.txt").exists() {
        tech_stack.push("Python");
    }
    if path.join("go.mod").exists() {
        tech_stack.push("Go");
    }

    let stack_str = if tech_stack.is_empty() {
        "Não especificada".to_string()
    } else {
        tech_stack.join(", ")
    };

    // Cut at a char boundary: slicing bytes panics inside a multi-byte character.
    let truncated_readme = match readme_content.char_indices().nth(1500) {
        Some((cut, _)) => format!("{}...", &readme_content[..cut]),
        None => readme_content,
    };

    let context_markdown = format!(
        "# Contexto Inicial do Projeto (Alethe AI Briefing)\n\n\
        > Este repositório foi clonado via Alethe. O briefing abaixo descreve a aplicação para orientar sua assistência.\n\n\
        ## Tecnologias Detectadas\n- {stack_str}\n\n\
        ## Resumo do Repositório (README)\n```markdown\n{truncated_readme}\n```\n\n\
        ## Instrução Importante para o Agente\n\
        Ao iniciar a conversa com o usuário neste repositório, faça um resumo executivo direto de 2 a 3 frases explicando o propósito deste projeto.\n"
    );

    // Never overwrite the repository's own agent guides.
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let file = path.join(name);
        if !file.exists() {
            let _ = fs::write(file, &context_markdown);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("alethe-projects-{label}-{}", nanoid::nanoid!(8)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn two_spellings_of_one_repository_compare_equal() {
        let plain = repo_identity("https://github.com/Owner/Repo.git");
        assert_eq!(plain, "github.com/owner/repo");
        for same in [
            "https://github.com/owner/repo",
            "http://www.github.com/owner/repo/",
            "git@github.com:owner/repo.git",
            "https://user:secret@github.com/owner/repo.git",
            "  HTTPS://GITHUB.COM/OWNER/REPO  ",
        ] {
            assert_eq!(repo_identity(same), plain, "{same}");
        }
        assert_ne!(repo_identity("https://github.com/owner/other"), plain);
        assert_ne!(repo_identity("https://github.com/someone/repo"), plain);
    }

    fn git_repo_with_origin(label: &str, origin: &str) -> PathBuf {
        let dir = temp_dir(label);
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .expect("git runs");
            assert!(status.status.success(), "git {args:?}");
        };
        git(&["init", "-q"]);
        git(&["remote", "add", "origin", origin]);
        dir
    }

    #[test]
    fn a_clone_target_is_free_taken_or_already_the_repository() {
        let url = "https://github.com/owner/repo";

        let missing = std::env::temp_dir().join(format!("alethe-missing-{}", nanoid::nanoid!(8)));
        assert_eq!(clone_target_state(&missing, url), CloneTarget::Free);

        let empty = temp_dir("empty");
        assert_eq!(clone_target_state(&empty, url), CloneTarget::Free);

        let busy = temp_dir("busy");
        fs::write(busy.join("notes.txt"), "mine").unwrap();
        assert_eq!(clone_target_state(&busy, url), CloneTarget::InUse);

        let same = git_repo_with_origin("same", "git@github.com:Owner/Repo.git");
        assert_eq!(clone_target_state(&same, url), CloneTarget::AlreadyCloned);

        let other = git_repo_with_origin("other", "https://github.com/owner/another");
        assert_eq!(clone_target_state(&other, url), CloneTarget::InUse);

        for dir in [empty, busy, same, other] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn normalizes_github_shorthands() {
        assert_eq!(
            normalize_github_url(" owner/repo "),
            "https://github.com/owner/repo"
        );
        assert_eq!(
            normalize_github_url("github.com/owner/repo"),
            "https://github.com/owner/repo"
        );
        assert_eq!(
            normalize_github_url("git@github.com:o/r.git"),
            "git@github.com:o/r.git"
        );
        assert_eq!(
            normalize_github_url("https://gitlab.com/o/r"),
            "https://gitlab.com/o/r"
        );
        assert_eq!(normalize_github_url("not a url"), "not a url");
    }

    #[test]
    fn folder_name_strips_git_suffix_and_sanitizes() {
        assert_eq!(
            repo_folder_name("https://github.com/o/my-repo.git"),
            "my-repo"
        );
        assert_eq!(repo_folder_name("git@github.com:o/r.git"), "r");
        assert_eq!(repo_folder_name("https://github.com/o/a b/"), "a_b");
        assert_eq!(repo_folder_name(".git"), "repo");
    }

    #[test]
    fn clone_target_appends_folder_unless_already_named() {
        let base = std::env::temp_dir();
        let joined =
            resolve_clone_target(base.to_str().unwrap(), "https://github.com/o/r").unwrap();
        assert_eq!(PathBuf::from(joined), base.join("r"));
        let named = base.join("r");
        let same = resolve_clone_target(named.to_str().unwrap(), "https://github.com/o/r").unwrap();
        assert_eq!(PathBuf::from(same), named);
    }

    #[test]
    fn external_todos_round_trip() {
        let dir = temp_dir("todos");
        let content = serde_json::json!({
            "preferences": { "todoStoragePath": dir.to_str().unwrap() },
            "todos": [{ "id": "t1" }],
        })
        .to_string();
        save_external_todos(&content).unwrap();

        let without_todos = serde_json::json!({
            "preferences": { "todoStoragePath": dir.to_str().unwrap() },
            "todos": [],
        })
        .to_string();
        let merged: Value = serde_json::from_str(&merge_external_todos(without_todos)).unwrap();
        assert_eq!(merged["todos"][0]["id"], "t1");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_without_external_path_returns_content_unchanged() {
        let content = r#"{"preferences":{"todoStoragePath":"  "},"todos":[]}"#.to_string();
        assert_eq!(merge_external_todos(content.clone()), content);
    }

    #[test]
    fn context_files_keep_existing_guides() {
        let dir = temp_dir("context");
        fs::write(dir.join("CLAUDE.md"), "own guide").unwrap();
        generate_repo_context_files(dir.to_str().unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
            "own guide"
        );
        assert!(dir.join("AGENTS.md").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn context_files_truncate_multibyte_readme_without_panicking() {
        let dir = temp_dir("readme");
        fs::write(dir.join("README.md"), "é".repeat(2000)).unwrap();
        generate_repo_context_files(dir.to_str().unwrap()).unwrap();
        let written = fs::read_to_string(dir.join("AGENTS.md")).unwrap();
        assert!(written.contains(&format!("{}...", "é".repeat(1500))));
        let _ = fs::remove_dir_all(dir);
    }
}
