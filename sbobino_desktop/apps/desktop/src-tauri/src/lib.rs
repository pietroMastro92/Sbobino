mod ai_support;
mod commands;
mod error;
mod parakeet_realtime;
mod realtime_audio;
mod release_assets;
mod state;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
#[cfg(target_os = "macos")]
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    Emitter,
};
use tokio::sync::{Mutex, Semaphore};
use tracing_subscriber::{fmt, EnvFilter};

use crate::commands::artifacts::{
    analyze_artifact_emotions, cancel_artifact_speaker_diarization, chat_artifact,
    delete_artifacts, empty_deleted_artifacts, export_artifact, generate_artifact_pack,
    get_artifact, hard_delete_artifacts, list_artifact_chat, list_artifacts,
    list_deleted_artifacts, list_recent_artifacts, optimize_artifact, preview_artifact_export,
    read_artifact_audio, read_audio_file, rename_artifact, restore_artifacts,
    run_artifact_speaker_diarization, summarize_artifact, update_artifact,
    update_artifact_timeline, write_trimmed_audio,
};
use crate::commands::automatic_import::{
    clear_automatic_import_quarantine_item, retry_automatic_import_quarantine_item,
    scan_automatic_import,
};
use crate::commands::backup::{export_app_backup, import_app_backup};
use crate::commands::companion::{
    companion_ai_status, companion_pairing_state, companion_status, revoke_companion_pairing,
    select_companion_ai_provider, start_companion, stop_companion,
};
use crate::commands::provisioning::{
    plan_pyannote_background_action, provisioning_cancel, provisioning_download_model,
    provisioning_install_pyannote, provisioning_install_runtime, provisioning_models,
    provisioning_start, provisioning_status, read_setup_report, reconcile_post_update_runtime,
    write_setup_report,
};
use crate::commands::realtime::{
    list_realtime_sessions, load_realtime_session, pause_realtime, resume_realtime, start_realtime,
    stop_realtime,
};
use crate::commands::runtime::{
    ensure_transcription_runtime, get_realtime_start_readiness, get_transcription_runtime_health,
    get_transcription_runtime_status, get_transcription_start_preflight,
};
use crate::commands::settings::{
    delete_prompt, get_ai_capability_status, get_ai_providers, get_settings, get_settings_snapshot,
    list_ai_service_models, list_gemini_models, list_prompts, list_transcription_languages,
    reset_prompts, save_prompt, test_ai_service, test_prompt, update_ai_providers, update_settings,
    update_settings_partial,
};
use crate::commands::transcription::{cancel_transcription, start_transcription};
use crate::commands::updates::check_updates;
use crate::commands::window::open_settings_window;
use crate::state::{ProvisioningRuntime, RealtimeRuntime};
use sbobino_infrastructure::adapters::whisper_stream::WhisperStreamEngine;

const PARAKEET_SAFE_METAL_ENV: [(&str, &str); 3] = [
    ("GGML_METAL_NO_RESIDENCY", "1"),
    ("GGML_METAL_SHARED_BUFFERS_DISABLE", "1"),
    ("GGML_METAL_CONCURRENCY_DISABLE", "1"),
];

#[cfg(target_os = "macos")]
const MENU_CHECK_UPDATES_ID: &str = "app_menu_check_updates";
#[cfg(target_os = "macos")]
const MENU_CHECK_UPDATES_EVENT: &str = "app://menu-check-updates";

fn configure_parakeet_metal_environment() {
    // This must run before any Parakeet CLI child process or in-process live
    // dylib is initialized. Keep Metal active, but force the ggml Metal safety
    // flags that made dev and packaged runtime behavior diverge on Apple
    // Silicon. CPU fallback remains an explicit diagnostic override only.
    for (name, value) in PARAKEET_SAFE_METAL_ENV {
        std::env::set_var(name, value);
    }
}

#[cfg(target_os = "macos")]
fn show_main_window(app_handle: &tauri::AppHandle) {
    if let Some(main_window) = app_handle.get_webview_window("main") {
        let _ = main_window.show();
        let _ = main_window.unminimize();
        let _ = main_window.set_focus();
    }
}

#[cfg(target_os = "macos")]
fn setup_macos_app_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    let check_updates_item = MenuItem::with_id(
        app,
        MENU_CHECK_UPDATES_ID,
        "Verifica disponibilita aggiornamenti...",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;

    if let Some(app_submenu) = menu
        .items()?
        .into_iter()
        .find_map(|item| item.as_submenu().cloned())
    {
        let insert_position = app_submenu.items()?.len().min(2);
        app_submenu.insert(&check_updates_item, insert_position)?;
        app_submenu.insert(&separator, insert_position + 1)?;
    } else {
        menu.append(&check_updates_item)?;
    }

    app.set_menu(menu)?;

    app.on_menu_event(|app_handle, event| {
        if event.id() != MENU_CHECK_UPDATES_ID {
            return;
        }
        show_main_window(app_handle);
        if let Some(main_window) = app_handle.get_webview_window("main") {
            let _ = main_window.emit(MENU_CHECK_UPDATES_EVENT, ());
        } else {
            let _ = app_handle.emit(MENU_CHECK_UPDATES_EVENT, ());
        }
    });

    Ok(())
}

pub fn run() {
    init_tracing();
    configure_parakeet_metal_environment();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Identity guard: fail closed before menu work, app data
            // initialisation, RuntimeFactory bootstrap, artifact storage and
            // secure storage (Keychain or local file backend).
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let bootstrap_profile =
                bootstrap_isolation::authorize_bootstrap(&bootstrap_isolation::BootstrapRequest {
                    identifier: &app.config().identifier,
                    data_dir: &data_dir,
                    support_dir: std::env::var_os(bootstrap_isolation::APP_SUPPORT_ENV)
                        .map(PathBuf::from),
                    local_secrets_opt_in: bootstrap_isolation::local_secrets_opt_in(
                        std::env::var_os(bootstrap_isolation::LOCAL_SECRETS_ENV).as_deref(),
                    ),
                    home: std::env::var_os("HOME").map(PathBuf::from),
                })
                .map_err(|error| {
                    tracing::error!("{error}");
                    std::io::Error::other(error)
                })?;

            #[cfg(target_os = "macos")]
            setup_macos_app_menu(app.handle())
                .map_err(|error| std::io::Error::other(format!("menu setup failure: {error}")))?;

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                use window_vibrancy::{
                    apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
                };
                let _ = apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::UnderWindowBackground,
                    Some(NSVisualEffectState::Active),
                    None,
                );
            }

            let resources_dir = app.path().resource_dir().ok();

            let bundle = sbobino_infrastructure::bootstrap(&data_dir, resources_dir)
                .map_err(|e| std::io::Error::other(format!("bootstrap failure: {e}")))?;

            // The local secrets backend must have produced its master key file
            // during the bootstrap; a candidate without it is not isolated.
            bootstrap_profile
                .verify_local_secrets_marker()
                .map_err(|error| {
                    tracing::error!("{error}");
                    std::io::Error::other(error)
                })?;

            let realtime_engine = bundle
                .runtime_factory
                .build_whisper_stream_engine()
                .unwrap_or_else(|_| {
                    let whisper_stream_path = bundle
                        .runtime_factory
                        .resolve_binary_path("whisper-stream", "whisper-stream");
                    let models_dir = bundle.runtime_factory.resolve_models_dir("models");
                    WhisperStreamEngine::new(whisper_stream_path, models_dir)
                });

            app.manage(state::AppState {
                companion: Arc::new(Mutex::new(None)),
                artifact_service: bundle.artifact_service,
                settings_service: bundle.settings_service,
                runtime_factory: bundle.runtime_factory,
                transcription_tasks: Arc::new(Mutex::new(HashMap::new())),
                diarization_tasks: Arc::new(Mutex::new(HashMap::new())),
                artifact_chat_locks: Arc::new(Mutex::new(HashMap::new())),
                transcription_gate: Arc::new(Semaphore::new(1)),
                realtime: RealtimeRuntime {
                    engine: Arc::new(Mutex::new(realtime_engine)),
                    parakeet_engine: Arc::new(Mutex::new(None)),
                    active_engine: Arc::new(Mutex::new(
                        sbobino_domain::TranscriptionEngine::WhisperCpp,
                    )),
                    active_job_id: Arc::new(Mutex::new(None)),
                    session_name: Arc::new(Mutex::new(None)),
                    model_filename: Arc::new(Mutex::new(None)),
                    language_code: Arc::new(Mutex::new("auto".to_string())),
                    model: Arc::new(Mutex::new(None)),
                    language: Arc::new(Mutex::new(None)),
                },
                provisioning: ProvisioningRuntime {
                    cancel_token: Arc::new(Mutex::new(None)),
                },
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_companion,
            stop_companion,
            companion_status,
            companion_pairing_state,
            revoke_companion_pairing,
            companion_ai_status,
            select_companion_ai_provider,
            get_settings,
            list_transcription_languages,
            update_settings,
            get_settings_snapshot,
            update_settings_partial,
            scan_automatic_import,
            retry_automatic_import_quarantine_item,
            clear_automatic_import_quarantine_item,
            get_ai_providers,
            get_ai_capability_status,
            update_ai_providers,
            list_gemini_models,
            list_ai_service_models,
            list_prompts,
            save_prompt,
            delete_prompt,
            reset_prompts,
            test_prompt,
            test_ai_service,
            start_transcription,
            cancel_transcription,
            run_artifact_speaker_diarization,
            cancel_artifact_speaker_diarization,
            list_artifacts,
            list_deleted_artifacts,
            list_recent_artifacts,
            get_artifact,
            list_artifact_chat,
            update_artifact,
            update_artifact_timeline,
            rename_artifact,
            delete_artifacts,
            restore_artifacts,
            hard_delete_artifacts,
            empty_deleted_artifacts,
            export_artifact,
            preview_artifact_export,
            export_app_backup,
            chat_artifact,
            summarize_artifact,
            generate_artifact_pack,
            analyze_artifact_emotions,
            optimize_artifact,
            read_artifact_audio,
            read_audio_file,
            write_trimmed_audio,
            import_app_backup,
            start_realtime,
            pause_realtime,
            resume_realtime,
            stop_realtime,
            list_realtime_sessions,
            load_realtime_session,
            provisioning_status,
            provisioning_models,
            plan_pyannote_background_action,
            reconcile_post_update_runtime,
            provisioning_start,
            provisioning_download_model,
            provisioning_install_pyannote,
            provisioning_install_runtime,
            provisioning_cancel,
            read_setup_report,
            write_setup_report,
            ensure_transcription_runtime,
            get_realtime_start_readiness,
            get_transcription_runtime_health,
            get_transcription_runtime_status,
            get_transcription_start_preflight,
            check_updates,
            open_settings_window,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build sbobino desktop app")
        .run(|_app_handle, _event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show_main_window(_app_handle);
            }
        });
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}

mod bootstrap_isolation {
    use std::ffi::{OsStr, OsString};
    use std::path::{Component, Path, PathBuf};

    /// Compiled product identifier. Only this identity keeps the historical
    /// behaviour (product data dir, Keychain, managed runtime).
    pub const OFFICIAL_IDENTIFIER: &str = "com.sbobino.desktop";
    /// Existing opt-in that switches secure storage to the local file backend.
    /// Candidates require the explicit value `1`; the official backend's
    /// historical parser is intentionally unchanged.
    pub const LOCAL_SECRETS_ENV: &str = "SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS";
    /// Existing isolated runtime support directory for verification launchers.
    pub const APP_SUPPORT_ENV: &str = "SBOBINO_APP_SUPPORT_DIR";
    /// Stable marker so a launcher can prove a bundle contains this guard.
    pub const GUARD_MARKER: &str = "bootstrap isolation guard";

    const SECURE_STORAGE_MARKER: &str = ".sbobino-secure-storage/master_key_v1";
    const PERSONAL_PROFILE_NAMES: [&str; 2] = ["com.sbobino.desktop", "sbobino"];

    pub fn local_secrets_opt_in(value: Option<&OsStr>) -> bool {
        value == Some(OsStr::new("1"))
    }

    pub struct BootstrapRequest<'a> {
        pub identifier: &'a str,
        pub data_dir: &'a Path,
        pub support_dir: Option<PathBuf>,
        pub local_secrets_opt_in: bool,
        pub home: Option<PathBuf>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BootstrapProfile {
        Official,
        IsolatedCandidate {
            data_dir: PathBuf,
            support_dir: PathBuf,
        },
    }

    impl BootstrapProfile {
        pub fn verify_local_secrets_marker(&self) -> Result<(), String> {
            let Self::IsolatedCandidate { data_dir, .. } = self else {
                return Ok(());
            };
            let marker = data_dir.join(SECURE_STORAGE_MARKER);
            if marker.is_file() {
                Ok(())
            } else {
                Err(refusal(format!(
                    "the local secrets backend did not create '{}'; refusing to run a \
                     non-official identity without verifiable isolated secure storage",
                    marker.display()
                )))
            }
        }
    }

    pub fn authorize_bootstrap(request: &BootstrapRequest<'_>) -> Result<BootstrapProfile, String> {
        let identifier = request.identifier.trim();
        if identifier.is_empty() {
            return Err(refusal(
                "the compiled application identifier is empty; refusing to bootstrap",
            ));
        }

        // The official product identity keeps the historical behaviour: product
        // data dir, Keychain, managed runtime and no new requirement.
        if identifier == OFFICIAL_IDENTIFIER {
            return Ok(BootstrapProfile::Official);
        }

        // Every other identity is a verification/candidate build. It may only
        // start with an explicit opt-in to the existing local secrets backend,
        // and never falls back to the product profile.
        if !request.local_secrets_opt_in {
            return Err(refusal(format!(
                "identifier '{identifier}' is not the official product identity \
                 '{OFFICIAL_IDENTIFIER}'; it requires an explicit local secrets opt-in \
                 ({LOCAL_SECRETS_ENV}=1) together with an isolated runtime support path \
                 ({APP_SUPPORT_ENV}=<isolated dir>). Refusing to bootstrap before settings, \
                 artifacts or Keychain access, and refusing to fall back to the product \
                 profile"
            )));
        }

        let home = request.home.as_deref().filter(|path| path.is_absolute()).ok_or_else(|| {
            refusal("a non-official identity requires an absolute home directory to exclude personal profiles")
        })?;
        let data_dir = require_absolute("candidate data dir", request.data_dir)?;
        reject_personal_profile_path(identifier, "candidate data dir", &data_dir, Some(home))?;

        // The bundled parakeet-cli wrapper falls back to the product application
        // support directory when this variable is absent, so an isolated
        // candidate has to declare it.
        let support_dir = match &request.support_dir {
            Some(path) if !path.as_os_str().is_empty() => require_absolute(APP_SUPPORT_ENV, path)?,
            _ => {
                return Err(refusal(format!(
                    "identifier '{identifier}' must declare {APP_SUPPORT_ENV}=<isolated dir>; \
                     the runtime wrapper would otherwise resolve the '{OFFICIAL_IDENTIFIER}' \
                     application support directory"
                )));
            }
        };
        reject_personal_profile_path(identifier, APP_SUPPORT_ENV, &support_dir, Some(home))?;

        // Only after every refusal path has passed do we create or restrict the
        // candidate profile, so a refused launch leaves the filesystem untouched.
        ensure_private_directory("candidate data dir", &data_dir)?;
        ensure_private_directory(APP_SUPPORT_ENV, &support_dir)?;

        Ok(BootstrapProfile::IsolatedCandidate {
            data_dir,
            support_dir,
        })
    }

    fn refusal(message: impl AsRef<str>) -> String {
        format!("{}: {}", GUARD_MARKER, message.as_ref())
    }

    fn resolve_existing_ancestors(path: &Path) -> PathBuf {
        let mut candidate = path.to_path_buf();
        let mut suffix: Vec<OsString> = Vec::new();
        loop {
            if let Ok(resolved) = std::fs::canonicalize(&candidate) {
                let mut output = resolved;
                for part in suffix.iter().rev() {
                    output.push(part);
                }
                return output;
            }
            let Some(name) = candidate.file_name().map(|name| name.to_os_string()) else {
                return path.to_path_buf();
            };
            let Some(parent) = candidate.parent() else {
                return path.to_path_buf();
            };
            if parent.as_os_str().is_empty() {
                return path.to_path_buf();
            }
            suffix.push(name);
            candidate = parent.to_path_buf();
        }
    }

    fn require_absolute(label: &str, path: &Path) -> Result<PathBuf, String> {
        if !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(refusal(format!(
                "{label} '{}' must be an absolute path without parent traversal; refusing \
                 a non-official identity with an unresolved path",
                path.display()
            )));
        }
        Ok(path.to_path_buf())
    }

    fn reject_personal_profile_path(
        identifier: &str,
        label: &str,
        path: &Path,
        home: Option<&Path>,
    ) -> Result<(), String> {
        let resolved = resolve_existing_ancestors(path);
        if let Some(name) = resolved.file_name().and_then(|name| name.to_str()) {
            if PERSONAL_PROFILE_NAMES
                .iter()
                .any(|personal| personal.eq_ignore_ascii_case(name))
            {
                return Err(refusal(format!(
                    "{label} '{}' resolves to the personal profile '{}'; refusing to touch \
                     product data from identifier '{identifier}'",
                    path.display(),
                    resolved.display()
                )));
            }
        }
        if let Some(home) = home {
            for personal in PERSONAL_PROFILE_NAMES {
                let personal_dir = resolve_existing_ancestors(
                    &home
                        .join("Library")
                        .join("Application Support")
                        .join(personal),
                );
                if resolved == personal_dir
                    || resolved.starts_with(&personal_dir)
                    || personal_dir.starts_with(&resolved)
                {
                    return Err(refusal(format!(
                        "{label} '{}' resolves inside the personal profile '{}'; refusing to \
                         touch product data from identifier '{identifier}'",
                        path.display(),
                        personal_dir.display()
                    )));
                }
            }
        }
        Ok(())
    }

    fn ensure_private_directory(label: &str, path: &Path) -> Result<(), String> {
        std::fs::create_dir_all(path).map_err(|error| {
            refusal(format!(
                "cannot create {label} '{}': {error}",
                path.display()
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    refusal(format!(
                        "{label} '{}' cannot be restricted to mode 0700: {error}",
                        path.display()
                    ))
                },
            )?;
            let mode = std::fs::metadata(path)
                .map_err(|error| {
                    refusal(format!(
                        "cannot inspect {label} '{}': {error}",
                        path.display()
                    ))
                })?
                .permissions()
                .mode()
                & 0o777;
            if mode != 0o700 {
                return Err(refusal(format!(
                    "{label} '{}' is mode {mode:04o} instead of 0700",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod bootstrap_isolation_tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::bootstrap_isolation::{
        authorize_bootstrap, BootstrapProfile, BootstrapRequest, APP_SUPPORT_ENV, GUARD_MARKER,
        LOCAL_SECRETS_ENV, OFFICIAL_IDENTIFIER,
    };

    const CANDIDATE_IDENTIFIER: &str = "com.sbobino.companion.baseline.isolated2";

    struct Fixture {
        temp: TempDir,
        home: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir");
            let home = temp.path().join("home");
            fs::create_dir_all(home.join("Library").join("Application Support"))
                .expect("fixture home");
            let fixture = Self { temp, home };
            for profile in [OFFICIAL_IDENTIFIER, "sbobino"] {
                let dir = fixture.application_support().join(profile);
                fs::create_dir_all(&dir).expect("personal profile");
                fs::set_permissions(&dir, fs::Permissions::from_mode(0o755))
                    .expect("personal profile mode");
            }
            fixture
        }

        fn application_support(&self) -> PathBuf {
            self.home.join("Library").join("Application Support")
        }

        fn product_profile(&self) -> PathBuf {
            self.application_support().join(OFFICIAL_IDENTIFIER)
        }

        fn legacy_profile(&self) -> PathBuf {
            self.application_support().join("sbobino")
        }

        /// Candidate-owned directory outside the synthetic home.
        fn isolated_root(&self) -> PathBuf {
            self.temp.path().join("candidate-root")
        }

        fn request<'a>(
            &'a self,
            identifier: &'a str,
            data_dir: &'a Path,
            support_dir: Option<PathBuf>,
            local_secrets_opt_in: bool,
        ) -> BootstrapRequest<'a> {
            BootstrapRequest {
                identifier,
                data_dir,
                support_dir,
                local_secrets_opt_in,
                home: Some(self.home.clone()),
            }
        }
    }

    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[test]
    fn non_canonical_identifier_without_local_secrets_opt_in_is_refused() {
        let fixture = Fixture::new();
        let data_dir = fixture.application_support().join(CANDIDATE_IDENTIFIER);
        let support_dir = fixture.isolated_root().join("app-support");

        let error = authorize_bootstrap(&fixture.request(
            CANDIDATE_IDENTIFIER,
            &data_dir,
            Some(support_dir.clone()),
            false,
        ))
        .expect_err("non-official identifier without opt-in must be refused");

        assert!(error.contains(GUARD_MARKER), "unexpected error: {error}");
        assert!(
            error.contains(LOCAL_SECRETS_ENV),
            "unexpected error: {error}"
        );
        assert!(
            error.contains(CANDIDATE_IDENTIFIER),
            "unexpected error: {error}"
        );
        assert!(
            !data_dir.exists(),
            "the guard must refuse before creating or touching the candidate data dir"
        );
        assert!(
            !support_dir.exists(),
            "the guard must refuse before creating the candidate support dir"
        );
        assert_eq!(
            mode_of(&fixture.product_profile()),
            0o755,
            "the product profile must stay untouched by the refusal path"
        );
    }

    #[test]
    fn non_canonical_identifier_with_opt_in_and_separated_paths_is_allowed() {
        let fixture = Fixture::new();
        let data_dir = fixture.application_support().join(CANDIDATE_IDENTIFIER);
        fs::create_dir_all(&data_dir).expect("pre-existing candidate data dir");
        fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o755)).expect("loose mode");
        let support_dir = fixture.isolated_root().join("app-support");

        let profile = authorize_bootstrap(&fixture.request(
            CANDIDATE_IDENTIFIER,
            &data_dir,
            Some(support_dir.clone()),
            true,
        ))
        .expect("explicit opt-in with separated paths is allowed");

        assert_eq!(
            profile,
            BootstrapProfile::IsolatedCandidate {
                data_dir: data_dir.clone(),
                support_dir: support_dir.clone(),
            }
        );
        assert_eq!(mode_of(&data_dir), 0o700, "candidate data dir must be 0700");
        assert_eq!(
            mode_of(&support_dir),
            0o700,
            "candidate support dir must be 0700"
        );
        assert_eq!(mode_of(&fixture.product_profile()), 0o755);
        assert_eq!(mode_of(&fixture.legacy_profile()), 0o755);
    }

    #[test]
    fn official_identifier_keeps_the_existing_behaviour() {
        let fixture = Fixture::new();
        let data_dir = fixture.product_profile();

        let profile =
            authorize_bootstrap(&fixture.request(OFFICIAL_IDENTIFIER, &data_dir, None, false))
                .expect("the official product identifier must always bootstrap");

        assert_eq!(profile, BootstrapProfile::Official);
        assert_eq!(
            mode_of(&fixture.product_profile()),
            0o755,
            "the official profile mode must not be rewritten"
        );
        assert!(
            !fixture
                .product_profile()
                .join(".sbobino-secure-storage")
                .exists(),
            "the official flow must not create a local secret backend"
        );
    }

    #[test]
    fn non_canonical_identifier_pointing_at_the_product_profile_is_refused() {
        let fixture = Fixture::new();
        let data_dir = fixture.product_profile();
        let support_dir = fixture.isolated_root().join("app-support");

        let error = authorize_bootstrap(&fixture.request(
            CANDIDATE_IDENTIFIER,
            &data_dir,
            Some(support_dir),
            true,
        ))
        .expect_err("a candidate identity may not use the product profile");

        assert!(error.contains(GUARD_MARKER), "unexpected error: {error}");
        assert_eq!(mode_of(&fixture.product_profile()), 0o755);
    }

    #[test]
    fn non_canonical_identifier_with_symlinked_product_profile_alias_is_refused() {
        let fixture = Fixture::new();
        let alias = fixture.application_support().join(CANDIDATE_IDENTIFIER);
        std::os::unix::fs::symlink(fixture.product_profile(), &alias).expect("symlink alias");
        let support_dir = fixture.isolated_root().join("app-support");

        let error = authorize_bootstrap(&fixture.request(
            CANDIDATE_IDENTIFIER,
            &alias,
            Some(support_dir),
            true,
        ))
        .expect_err("a symlink alias of the product profile must be refused");

        assert!(error.contains(GUARD_MARKER), "unexpected error: {error}");
        assert_eq!(
            mode_of(&fixture.product_profile()),
            0o755,
            "a symlink alias must be refused before any chmod reaches the product profile"
        );
    }

    #[test]
    fn non_canonical_identifier_with_support_dir_alias_of_legacy_profile_is_refused() {
        let fixture = Fixture::new();
        let data_dir = fixture.isolated_root().join("data");
        let support_alias = fixture.isolated_root().join("app-support");
        fs::create_dir_all(fixture.isolated_root()).expect("isolated root");
        std::os::unix::fs::symlink(fixture.legacy_profile(), &support_alias)
            .expect("symlink support alias");

        let error = authorize_bootstrap(&fixture.request(
            CANDIDATE_IDENTIFIER,
            &data_dir,
            Some(support_alias),
            true,
        ))
        .expect_err("a support dir aliased to the legacy profile must be refused");

        assert!(error.contains(GUARD_MARKER), "unexpected error: {error}");
        assert_eq!(mode_of(&fixture.legacy_profile()), 0o755);
    }

    #[test]
    fn non_canonical_identifier_with_support_dir_inside_product_profile_is_refused() {
        let fixture = Fixture::new();
        let data_dir = fixture.isolated_root().join("data");
        let support_dir = fixture.product_profile().join("verification-support");

        let error = authorize_bootstrap(&fixture.request(
            CANDIDATE_IDENTIFIER,
            &data_dir,
            Some(support_dir),
            true,
        ))
        .expect_err("a support dir inside the product profile must be refused");

        assert!(error.contains(GUARD_MARKER), "unexpected error: {error}");
        assert!(!fixture.isolated_root().join("data").exists());
    }

    #[test]
    fn non_canonical_identifier_without_support_dir_is_refused() {
        let fixture = Fixture::new();
        let data_dir = fixture.isolated_root().join("data");

        let error =
            authorize_bootstrap(&fixture.request(CANDIDATE_IDENTIFIER, &data_dir, None, true))
                .expect_err("a candidate identity must declare an isolated runtime support dir");

        assert!(error.contains(APP_SUPPORT_ENV), "unexpected error: {error}");
        assert!(!data_dir.exists());
    }

    #[test]
    fn non_canonical_identifier_with_relative_data_dir_is_refused() {
        let fixture = Fixture::new();
        let data_dir = PathBuf::from("relative-data");
        let support_dir = fixture.isolated_root().join("app-support");

        let error = authorize_bootstrap(&fixture.request(
            CANDIDATE_IDENTIFIER,
            &data_dir,
            Some(support_dir),
            true,
        ))
        .expect_err("an unresolved relative data dir must be refused");

        assert!(error.contains(GUARD_MARKER), "unexpected error: {error}");
    }

    #[test]
    fn isolated_candidate_must_prove_the_local_secrets_backend_after_bootstrap() {
        let fixture = Fixture::new();
        let data_dir = fixture.isolated_root().join("data");
        let support_dir = fixture.isolated_root().join("app-support");
        let profile = authorize_bootstrap(&fixture.request(
            CANDIDATE_IDENTIFIER,
            &data_dir,
            Some(support_dir),
            true,
        ))
        .expect("isolated candidate plan");

        let error = profile
            .verify_local_secrets_marker()
            .expect_err("a candidate without a local master key must fail closed");
        assert!(error.contains(GUARD_MARKER), "unexpected error: {error}");

        let marker = data_dir
            .join(".sbobino-secure-storage")
            .join("master_key_v1");
        fs::create_dir_all(marker.parent().expect("marker parent")).expect("secure storage dir");
        fs::write(&marker, b"00").expect("marker placeholder");
        profile
            .verify_local_secrets_marker()
            .expect("an existing local master key marker certifies the isolated backend");

        let official = authorize_bootstrap(&fixture.request(
            OFFICIAL_IDENTIFIER,
            &fixture.product_profile(),
            None,
            false,
        ))
        .expect("official plan");
        official
            .verify_local_secrets_marker()
            .expect("the official Keychain flow has no local marker to prove");
    }

    #[test]
    fn candidate_local_secrets_opt_in_requires_the_explicit_value_one() {
        use std::ffi::OsStr;
        assert!(!super::bootstrap_isolation::local_secrets_opt_in(None));
        for value in ["", "0", "false", "true", "yes"] {
            assert!(!super::bootstrap_isolation::local_secrets_opt_in(Some(
                OsStr::new(value)
            )));
        }
        assert!(super::bootstrap_isolation::local_secrets_opt_in(Some(
            OsStr::new("1")
        )));
    }

    #[test]
    fn candidate_without_known_home_is_refused_before_creating_data() {
        let fixture = Fixture::new();
        let data_dir = fixture.isolated_root().join("data");
        let support_dir = fixture.isolated_root().join("support");
        let mut request = fixture.request(
            CANDIDATE_IDENTIFIER,
            &data_dir,
            Some(support_dir.clone()),
            true,
        );
        request.home = None;
        assert!(authorize_bootstrap(&request).is_err());
        assert!(!data_dir.exists());
        assert!(!support_dir.exists());
    }

    #[test]
    fn candidate_parent_traversal_and_personal_profile_ancestors_are_refused() {
        let fixture = Fixture::new();
        let data_dir = fixture.isolated_root().join("data");
        for support_dir in [
            fixture.isolated_root().join("missing/../support"),
            fixture.application_support(),
        ] {
            let request = fixture.request(CANDIDATE_IDENTIFIER, &data_dir, Some(support_dir), true);
            assert!(authorize_bootstrap(&request).is_err());
            assert!(!data_dir.exists());
            assert_eq!(mode_of(&fixture.product_profile()), 0o755);
        }
    }

    #[test]
    fn setup_authorises_the_profile_before_infrastructure_bootstrap() {
        let source = include_str!("lib.rs");

        let setup = source.find(".setup(|app|").expect("tauri setup closure");
        let guard = source
            .find("bootstrap_isolation::authorize_bootstrap")
            .expect("setup must call the isolation guard");
        let bootstrap = source
            .find("sbobino_infrastructure::bootstrap(")
            .expect("setup must call the infrastructure bootstrap");
        let marker = source
            .find(".verify_local_secrets_marker()")
            .expect("setup must verify the local secrets marker");

        assert!(
            setup < guard && guard < bootstrap,
            "the isolation guard must run inside setup before the infrastructure bootstrap"
        );
        assert!(
            bootstrap < marker,
            "the local secrets marker can only be verified after the bootstrap"
        );

        let data_dir = source
            .find("app_data_dir()")
            .expect("app data dir resolution");
        assert!(
            setup < data_dir && data_dir < guard,
            "the guard must consume the resolved app data dir before any bootstrap step"
        );
    }
}
