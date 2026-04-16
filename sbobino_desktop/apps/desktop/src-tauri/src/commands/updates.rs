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

fn select_manual_download_asset<'a>(
    latest_version: &str,
    assets: &'a [GitHubAsset],
) -> Option<&'a GitHubAsset> {
    let expected_dmg = format!("Sbobino_{latest_version}_aarch64.dmg");
    assets.iter().find(|asset| asset.name == expected_dmg)
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

    let download_url = select_manual_download_asset(&latest_version, &release.assets)
        .map(|asset| asset.browser_download_url.clone());

    Ok(UpdateCheckResponse {
        has_update,
        current_version,
        latest_version: Some(latest_version),
        download_url,
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

#[cfg(test)]
mod tests {
    use super::{compare_versions, select_manual_download_asset, GitHubAsset};

    #[test]
    fn manual_download_prefers_exact_apple_silicon_dmg() {
        let assets = vec![
            GitHubAsset {
                name: "speech-runtime-macos-aarch64.zip".to_string(),
                browser_download_url: "https://example.com/runtime.zip".to_string(),
            },
            GitHubAsset {
                name: "Sbobino_0.1.16_aarch64.dmg".to_string(),
                browser_download_url: "https://example.com/Sbobino_0.1.16_aarch64.dmg".to_string(),
            },
            GitHubAsset {
                name: "pyannote-runtime-macos-aarch64.zip".to_string(),
                browser_download_url: "https://example.com/pyannote.zip".to_string(),
            },
        ];

        let selected =
            select_manual_download_asset("0.1.16", &assets).expect("expected a dmg asset");
        assert_eq!(selected.name, "Sbobino_0.1.16_aarch64.dmg");
    }

    #[test]
    fn manual_download_does_not_fall_back_to_runtime_zip_assets() {
        let assets = vec![
            GitHubAsset {
                name: "speech-runtime-macos-aarch64.zip".to_string(),
                browser_download_url: "https://example.com/runtime.zip".to_string(),
            },
            GitHubAsset {
                name: "pyannote-runtime-macos-aarch64.zip".to_string(),
                browser_download_url: "https://example.com/pyannote.zip".to_string(),
            },
        ];

        assert!(select_manual_download_asset("0.1.16", &assets).is_none());
    }

    #[test]
    fn compare_versions_handles_patch_updates() {
        assert_eq!(compare_versions("0.1.16", "0.1.15"), 1);
        assert_eq!(compare_versions("0.1.13", "0.1.13"), 0);
        assert_eq!(compare_versions("0.1.12", "0.1.13"), -1);
    }
}
