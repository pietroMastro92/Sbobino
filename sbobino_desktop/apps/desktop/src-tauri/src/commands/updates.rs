use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{error::CommandError, release_assets::production_release_repository, state::AppState};

#[derive(Debug, Serialize)]
pub struct UpdateCheckResponse {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub download_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[tauri::command]
pub async fn check_updates(
    _state: State<'_, AppState>,
) -> Result<UpdateCheckResponse, CommandError> {
    let repo = production_release_repository();
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");

    let current_version = env!("CARGO_PKG_VERSION").to_string();

    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "sbobino-desktop")
        .send()
        .await
        .map_err(|e| CommandError::new("update", format!("failed to check updates: {e}")))?
        .error_for_status()
        .map_err(|e| CommandError::new("update", format!("update endpoint failed: {e}")))?;

    let release = response
        .json::<GitHubRelease>()
        .await
        .map_err(|e| CommandError::new("update", format!("invalid update response: {e}")))?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let has_update = compare_versions(&latest_version, &current_version) > 0;

    let dmg_or_zip = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(".dmg") || asset.name.ends_with(".zip"))
        .map(|asset| asset.browser_download_url.clone());

    Ok(UpdateCheckResponse {
        has_update,
        current_version,
        latest_version: Some(latest_version),
        download_url: dmg_or_zip,
    })
}

fn compare_versions(a: &str, b: &str) -> i32 {
    let parse = |version: &str| {
        version
            .split('.')
            .map(|part| part.parse::<i32>().unwrap_or(0))
            .collect::<Vec<_>>()
    };

    let mut lhs = parse(a);
    let mut rhs = parse(b);

    let max_len = lhs.len().max(rhs.len());
    lhs.resize(max_len, 0);
    rhs.resize(max_len, 0);

    for (left, right) in lhs.iter().zip(rhs.iter()) {
        if left > right {
            return 1;
        }
        if left < right {
            return -1;
        }
    }

    0
}
