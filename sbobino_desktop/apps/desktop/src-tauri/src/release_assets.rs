use serde::{Deserialize, Serialize};

pub const PRODUCTION_RELEASE_REPOSITORY: &str = "pietroMastro92/Sbobino";
pub const SETUP_MANIFEST_ASSET: &str = "setup-manifest.json";
pub const RUNTIME_MANIFEST_ASSET: &str = "runtime-manifest.json";
pub const RUNTIME_AARCH64_ASSET: &str = "speech-runtime-macos-aarch64.zip";
pub const PYANNOTE_MANIFEST_ASSET: &str = "pyannote-manifest.json";
pub const PYANNOTE_RUNTIME_AARCH64_ASSET: &str = "pyannote-runtime-macos-aarch64.zip";
pub const PYANNOTE_RUNTIME_X86_64_ASSET: &str = "pyannote-runtime-macos-x86_64.zip";
pub const PYANNOTE_MODEL_ASSET: &str = "pyannote-model-community-1.zip";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseAssetDescriptor {
    pub name: String,
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub expanded_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupReleaseManifest {
    pub app_version: String,
    pub release_tag: String,
    pub runtime_manifest: ReleaseAssetDescriptor,
    pub runtime_asset: ReleaseAssetDescriptor,
    pub pyannote_manifest: ReleaseAssetDescriptor,
    pub pyannote_runtime_asset: ReleaseAssetDescriptor,
    pub pyannote_model_asset: ReleaseAssetDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeReleaseManifest {
    pub app_version: String,
    pub assets: Vec<RuntimeReleaseAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeReleaseAsset {
    pub kind: String,
    pub name: String,
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub expanded_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PyannoteReleaseManifest {
    pub app_version: String,
    pub assets: Vec<PyannoteReleaseAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PyannoteReleaseAsset {
    pub kind: String,
    pub name: String,
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub expanded_size_bytes: Option<u64>,
}

pub fn production_release_repository() -> &'static str {
    PRODUCTION_RELEASE_REPOSITORY
}

pub fn release_tag(version: &str) -> String {
    format!("v{version}")
}

pub fn release_asset_url(version: &str, asset_name: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/{}/{}",
        production_release_repository(),
        release_tag(version),
        asset_name
    )
}
