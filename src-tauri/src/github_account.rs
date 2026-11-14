//! GitHub account sign-in (OAuth device flow) used to browse and clone the user's repositories.
//!
//! The access token lives in the OS secret store, never in a file. Only the public profile (login,
//! name, avatar) is kept in `github_account.json` next to the profile data.

use std::fs;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::paths::app_data_dir;
use crate::provider_common::now_ms;
use crate::secure_store::{OsSecretStore, SecretKind, SecretStore};

const USER_AGENT: &str = "Arcadia";
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const API_BASE: &str = "https://api.github.com";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// `repo` lets the app list and clone private repositories; `read:user` reads the profile.
const SCOPE: &str = "repo read:user";
/// Client ID of the Arcadia OAuth App (public by design, device flow enabled, tokens do not
/// expire). `ARCADIA_GITHUB_CLIENT_ID` overrides it at run time or build time.
const DEFAULT_CLIENT_ID: &str = "Ov23liylvJy9hYy3MW28";
const CONFIG_FILE: &str = "github_account.json";
const REPOS_PER_PAGE: usize = 100;
const MAX_PAGES: usize = 3;

fn client_id() -> Option<String> {
    std::env::var("ARCADIA_GITHUB_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("ARCADIA_GITHUB_CLIENT_ID").map(str::to_string))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| (!DEFAULT_CLIENT_ID.is_empty()).then(|| DEFAULT_CLIENT_ID.to_string()))
}

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct StoredProfile {
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
}

#[derive(Serialize)]
pub struct GithubAccountStatus {
    /// False when the build has no OAuth App client ID, so sign-in cannot start.
    pub configured: bool,
    pub connected: bool,
    pub login: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Serialize)]
pub struct GithubDeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct GithubRepo {
    pub id: u64,
    pub full_name: String,
    pub name: String,
    pub owner: String,
    pub description: Option<String>,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
    pub updated_at: Option<String>,
    pub clone_url: String,
}

fn profile_id(app: &AppHandle) -> Result<String, String> {
    Ok(crate::profiles::ensure_profiles_index(app)?.active_profile_id)
}

fn load_profile(app: &AppHandle) -> StoredProfile {
    app_data_dir(app)
        .ok()
        .and_then(|dir| fs::read_to_string(dir.join(CONFIG_FILE)).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_profile(app: &AppHandle, profile: &StoredProfile) -> Result<(), String> {
    let dir = app_data_dir(app)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let raw = serde_json::to_string_pretty(profile).map_err(|e| e.to_string())?;
    let path = dir.join(CONFIG_FILE);
    let tmp = path.with_extension(format!("json.{}.tmp", nanoid::nanoid!(8)));
    fs::write(&tmp, raw).map_err(|e| e.to_string())?;
    if let Err(error) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(error.to_string());
    }
    Ok(())
}

fn stored_token(app: &AppHandle) -> Option<String> {
    let id = profile_id(app).ok()?;
    OsSecretStore
        .get(&id, SecretKind::GithubAccountToken)
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty())
}

/// Token to authenticate a `git clone` of a github.com repository, when the user is signed in.
pub(crate) fn token_for_clone(app: &AppHandle) -> Option<String> {
    stored_token(app)
}

/// Environment that makes git send the token to github.com only, without putting it in the
/// command line or the remote URL.
pub(crate) fn git_auth_env(token: &str) -> Vec<(String, String)> {
    use base64::Engine;
    let credentials =
        base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
    vec![
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        (
            "GIT_CONFIG_KEY_0".into(),
            "http.https://github.com/.extraheader".into(),
        ),
        (
            "GIT_CONFIG_VALUE_0".into(),
            format!("AUTHORIZATION: basic {credentials}"),
        ),
    ]
}

fn status_of(app: &AppHandle) -> GithubAccountStatus {
    let profile = load_profile(app);
    GithubAccountStatus {
        configured: client_id().is_some(),
        connected: stored_token(app).is_some(),
        login: profile.login,
        name: profile.name,
        avatar_url: profile.avatar_url,
    }
}

fn clear_session(app: &AppHandle) -> Result<(), String> {
    OsSecretStore
        .delete(&profile_id(app)?, SecretKind::GithubAccountToken)
        .map_err(|e| e.to_string())?;
    save_profile(app, &StoredProfile::default())
}

#[tauri::command]
pub fn github_account_status(app: AppHandle) -> GithubAccountStatus {
    status_of(&app)
}

#[tauri::command]
pub async fn github_account_device_start() -> Result<GithubDeviceStart, String> {
    let client_id = client_id().ok_or_else(|| "client_id_missing".to_string())?;
    let resp = http()
        .post(DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .json(&json!({ "client_id": client_id, "scope": SCOPE }))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("github returned {}", resp.status()));
    }
    let body: Value = resp.json().await.map_err(|e| format!("json parse: {e}"))?;
    if let Some(error) = body.get("error").and_then(Value::as_str) {
        return Err(format!("github oauth error: {error}"));
    }
    let field = |name: &str| -> Result<String, String> {
        body.get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("device response missing {name}"))
    };
    Ok(GithubDeviceStart {
        device_code: field("device_code")?,
        user_code: field("user_code")?,
        verification_uri: field("verification_uri")?,
        interval: body.get("interval").and_then(Value::as_u64).unwrap_or(5),
        expires_in: body
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(900),
    })
}

async fn poll_token(
    client_id: &str,
    device_code: &str,
    interval: u64,
    expires_in: u64,
) -> Result<String, String> {
    let mut wait = interval.max(5);
    let deadline = now_ms() + expires_in.min(900) * 1000;
    loop {
        if now_ms() > deadline {
            return Err("code_expired".to_string());
        }
        tokio::time::sleep(Duration::from_secs(wait)).await;
        let resp = http()
            .post(ACCESS_TOKEN_URL)
            .header("Accept", "application/json")
            .json(&json!({
                "client_id": client_id,
                "device_code": device_code,
                "grant_type": DEVICE_GRANT_TYPE,
            }))
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("github returned {}", resp.status()));
        }
        let body: Value = resp.json().await.map_err(|e| format!("json parse: {e}"))?;
        if let Some(token) = body.get("access_token").and_then(Value::as_str) {
            return Ok(token.to_string());
        }
        match body.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => continue,
            Some("slow_down") => wait += 5,
            Some("access_denied") => return Err("access_denied".to_string()),
            Some("expired_token") => return Err("code_expired".to_string()),
            Some(other) => return Err(format!("github oauth error: {other}")),
            None => return Err("malformed oauth response".to_string()),
        }
    }
}

async fn api_get(token: &str, url: &str) -> Result<Value, String> {
    let resp = http()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 {
        return Err("session_expired".to_string());
    }
    if !(200..300).contains(&status) {
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        return Err(api_error(status, &body));
    }
    resp.json().await.map_err(|e| format!("json parse: {e}"))
}

/// "github returned 403: <GitHub's own message>", so the reason reaches the user.
fn api_error(status: u16, body: &Value) -> String {
    match body.get("message").and_then(Value::as_str) {
        Some(message) if !message.trim().is_empty() => {
            format!("github returned {status}: {message}")
        }
        _ => format!("github returned {status}"),
    }
}

#[tauri::command]
pub async fn github_account_device_finish(
    app: AppHandle,
    device_code: String,
    interval: u64,
    expires_in: u64,
) -> Result<GithubAccountStatus, String> {
    let client_id = client_id().ok_or_else(|| "client_id_missing".to_string())?;
    let token = poll_token(&client_id, &device_code, interval, expires_in).await?;
    let user = api_get(&token, &format!("{API_BASE}/user")).await?;
    OsSecretStore
        .set(&profile_id(&app)?, SecretKind::GithubAccountToken, &token)
        .map_err(|e| e.to_string())?;
    let text = |name: &str| user.get(name).and_then(Value::as_str).map(str::to_string);
    save_profile(
        &app,
        &StoredProfile {
            login: text("login"),
            name: text("name"),
            avatar_url: text("avatar_url"),
        },
    )?;
    Ok(status_of(&app))
}

#[tauri::command]
pub fn github_account_logout(app: AppHandle) -> Result<GithubAccountStatus, String> {
    clear_session(&app)?;
    Ok(status_of(&app))
}

pub(crate) fn parse_repos(page: &Value) -> Vec<GithubRepo> {
    page.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let text =
                        |name: &str| item.get(name).and_then(Value::as_str).map(str::to_string);
                    Some(GithubRepo {
                        id: item.get("id")?.as_u64()?,
                        full_name: text("full_name")?,
                        name: text("name")?,
                        owner: item.get("owner")?.get("login")?.as_str()?.to_string(),
                        description: text("description"),
                        private: item
                            .get("private")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        fork: item.get("fork").and_then(Value::as_bool).unwrap_or(false),
                        archived: item
                            .get("archived")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        updated_at: text("pushed_at").or_else(|| text("updated_at")),
                        clone_url: text("clone_url")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Pages of `/user/repos` for one affiliation filter.
async fn list_repos(token: &str, affiliation: &str) -> Result<Vec<GithubRepo>, String> {
    let mut repos = Vec::new();
    for page in 1..=MAX_PAGES {
        let url = format!(
            "{API_BASE}/user/repos?per_page={REPOS_PER_PAGE}&page={page}&sort=pushed             &affiliation={affiliation}"
        );
        let batch = parse_repos(&api_get(token, &url).await?);
        let full = batch.len() >= REPOS_PER_PAGE;
        repos.extend(batch);
        if !full {
            break;
        }
    }
    Ok(repos)
}

/// The signed-in user's repositories (own, collaborator and organization), newest activity first.
/// Organizations that restrict OAuth apps or enforce SAML can answer 403 to the full listing; the
/// user's own and collaborator repositories are listed anyway.
#[tauri::command]
pub async fn github_account_repos(app: AppHandle) -> Result<Vec<GithubRepo>, String> {
    let token = stored_token(&app).ok_or_else(|| "not_connected".to_string())?;
    let result = match list_repos(&token, "owner,collaborator,organization_member").await {
        Err(error) if error.starts_with("github returned 403") => {
            list_repos(&token, "owner,collaborator").await
        }
        other => other,
    };
    if matches!(&result, Err(error) if error == "session_expired") {
        let _ = clear_session(&app);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repositories_and_skips_malformed_entries() {
        let repos = parse_repos(&json!([
            {
                "id": 1,
                "full_name": "me/app",
                "name": "app",
                "owner": { "login": "me" },
                "description": "An app",
                "private": true,
                "fork": false,
                "archived": false,
                "pushed_at": "2026-11-01T10:00:00Z",
                "updated_at": "2026-10-01T10:00:00Z",
                "clone_url": "https://github.com/me/app.git"
            },
            { "id": 2, "name": "broken" }
        ]));

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].full_name, "me/app");
        assert_eq!(repos[0].owner, "me");
        assert!(repos[0].private);
        assert_eq!(repos[0].updated_at.as_deref(), Some("2026-11-01T10:00:00Z"));
        assert_eq!(repos[0].clone_url, "https://github.com/me/app.git");
    }

    #[test]
    fn api_errors_carry_the_message_github_sent() {
        assert_eq!(
            api_error(
                403,
                &json!({ "message": "Resource protected by organization SAML enforcement." })
            ),
            "github returned 403: Resource protected by organization SAML enforcement."
        );
        assert_eq!(api_error(500, &Value::Null), "github returned 500");
    }

    #[test]
    fn a_non_array_response_has_no_repositories() {
        assert!(parse_repos(&json!({ "message": "Bad credentials" })).is_empty());
    }

    #[test]
    fn git_auth_is_scoped_to_github_and_kept_out_of_the_arguments() {
        let env = git_auth_env("secret-token");
        let get = |key: &str| {
            env.iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(get("GIT_TERMINAL_PROMPT").as_deref(), Some("0"));
        assert_eq!(
            get("GIT_CONFIG_KEY_0").as_deref(),
            Some("http.https://github.com/.extraheader")
        );
        let header = get("GIT_CONFIG_VALUE_0").unwrap();
        assert!(header.starts_with("AUTHORIZATION: basic "));
        assert!(!header.contains("secret-token"));
    }
}
