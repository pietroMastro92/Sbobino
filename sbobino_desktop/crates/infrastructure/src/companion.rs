//! Opt-in TLS companion. Reuses the desktop service and its heavy-work gate.
#[path = "companion_ai.rs"]
mod ai;
use crate::RuntimeTranscriptionFactory;
pub use ai::{AiService, AiSettingsStatus};
use base64::{engine::general_purpose::STANDARD, Engine};
use http_body_util::{BodyExt, Full};
use hyper::{
    body::{Bytes, Incoming},
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use sbobino_application::{ApplicationError, RunTranscriptionRequest};
use sbobino_domain::{AppSettings, ArtifactSourceOrigin, TranscriptArtifact};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    convert::Infallible,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{io::AsyncWriteExt, net::TcpListener, sync::Semaphore};
use tokio_rustls::{
    rustls::{self, pki_types::PrivatePkcs8KeyDer},
    TlsAcceptor,
};
use tokio_util::sync::CancellationToken;

const MAX_UPLOAD: u64 = 2 * 1024 * 1024 * 1024;
const MAX_JOBS: usize = 8;
const REPLACE: &str = "\u{001f}REPLACE:";

#[derive(Clone, Serialize, Deserialize)]
pub struct Pairing {
    pub version: u8,
    pub endpoint: String,
    pub token: String,
    pub certificate: String,
    pub desktop_name: String,
    pub desktop_version: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub id: String,
    pub status: String,
    pub text: String,
    pub message: String,
    pub percentage: u8,
    pub artifact_id: Option<String>,
    pub duration: Option<f32>,
    pub engine: Option<String>,
    pub model: Option<String>,
    /// Slim terminal timeline. Per-word timings remain in the desktop
    /// artifact and are intentionally not duplicated over the companion wire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline_v2: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diarization_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diarization_error: Option<String>,
}

struct Job {
    snapshot: JobSnapshot,
    cancel: CancellationToken,
}

struct ServerState {
    ai: Arc<AiService>,
    token: String,
    factory: Arc<RuntimeTranscriptionFactory>,
    gate: Arc<Semaphore>,
    jobs: Mutex<HashMap<String, Job>>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    stop: CancellationToken,
    completed: Arc<dyn Fn(TranscriptArtifact) + Send + Sync>,
}

pub struct CompanionServer {
    pub pairing: Pairing,
    ai: Arc<AiService>,
    stop: CancellationToken,
    accept_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for CompanionServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl CompanionServer {
    pub async fn shutdown(&self) {
        self.stop.cancel();
        if let Some(task) = self.accept_task.lock().await.take() {
            let _ = task.await;
        }
        self.ai.shutdown().await;
    }
    pub fn ai_service(&self) -> Arc<AiService> {
        self.ai.clone()
    }

    pub async fn start(
        factory: Arc<RuntimeTranscriptionFactory>,
        gate: Arc<Semaphore>,
        lan: bool,
        desktop_version: String,
        completed: Arc<dyn Fn(TranscriptArtifact) + Send + Sync>,
    ) -> Result<Self, String> {
        let advertised_ip = if lan {
            local_ip()
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        let bind_ip = if lan {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            advertised_ip
        };
        let listener = TcpListener::bind((bind_ip, 0))
            .await
            .map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let certified = rcgen::generate_simple_self_signed(vec![
            advertised_ip.to_string(),
            "localhost".into(),
            "127.0.0.1".into(),
        ])
        .map_err(|e| e.to_string())?;
        let der = certified.cert.der().clone();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(
                vec![der.clone()],
                PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()).into(),
            )
            .map_err(|e| e.to_string())?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let stop = CancellationToken::new();
        let ai =
            AiService::new(factory.clone(), gate.clone(), stop.clone()).unwrap_or_else(|reason| {
                AiService::disabled(factory.clone(), gate.clone(), stop.clone(), reason)
            });
        let state = Arc::new(ServerState {
            ai: ai.clone(),
            token: token.clone(),
            factory,
            gate,
            jobs: Mutex::new(HashMap::new()),
            tasks: Mutex::new(Vec::new()),
            stop: stop.clone(),
            completed,
        });
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let connections = Arc::new(Semaphore::new(8));
        let accept_task = tokio::spawn(async move {
            let mut connections_tasks = tokio::task::JoinSet::new();
            loop {
                while connections_tasks.try_join_next().is_some() {}
                let accepted = tokio::select! {
                    _ = state.stop.cancelled() => break,
                    value = listener.accept() => value,
                };
                let Ok((stream, _)) = accepted else { break };
                let Ok(permit) = connections.clone().try_acquire_owned() else {
                    continue;
                };
                let acceptor = acceptor.clone();
                let state = state.clone();
                connections_tasks.spawn(async move {
                    let _permit = permit;
                    let handshake=tokio::select! {_=state.stop.cancelled()=>return, value=tokio::time::timeout(Duration::from_secs(10),acceptor.accept(stream))=>value};
                    let Ok(Ok(tls)) = handshake
                    else {
                        return;
                    };
                    let stop = state.stop.clone();
                    let connection = hyper::server::conn::http1::Builder::new()
                        .keep_alive(false)
                        .serve_connection(
                            TokioIo::new(tls),
                            service_fn(move |request| handle(request, state.clone())),
                        );
                    tokio::select! { _ = stop.cancelled() => {}, _ = connection => {} }
                });
            }
            connections_tasks.abort_all();
            while connections_tasks.join_next().await.is_some() {}
            for job in state.jobs.lock().unwrap().values() {
                job.cancel.cancel();
            }
            let tasks = std::mem::take(&mut *state.tasks.lock().unwrap());
            for task in tasks {
                let _ = task.await;
            }
        });
        Ok(Self {
            accept_task: tokio::sync::Mutex::new(Some(accept_task)),
            ai,
            pairing: Pairing {
                version: 1,
                endpoint: format!("https://{advertised_ip}:{port}"),
                token,
                certificate: STANDARD.encode(der.as_ref()),
                desktop_name: "Sbobino Desktop".into(),
                desktop_version,
            },
            stop,
        })
    }
}

fn local_ip() -> IpAddr {
    // UDP connect only selects a local route; no packets or audio are sent.
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn response(status: StatusCode, value: serde_json::Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-store")
        .body(Full::new(Bytes::from(value.to_string())))
        .unwrap()
}

fn snapshot(state: &ServerState, id: &str) -> Option<JobSnapshot> {
    state
        .jobs
        .lock()
        .unwrap()
        .get(id)
        .map(|j| j.snapshot.clone())
}

fn update(state: &ServerState, id: &str, f: impl FnOnce(&mut JobSnapshot)) {
    if let Some(job) = state.jobs.lock().unwrap().get_mut(id) {
        f(&mut job.snapshot);
    }
}

async fn handle(
    mut req: Request<Incoming>,
    state: Arc<ServerState>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let supplied = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let expected = format!("Bearer {}", state.token);
    if !constant_time_equal(supplied.as_bytes(), expected.as_bytes()) {
        return Ok(response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({"message":"Abbinamento richiesto."}),
        ));
    }
    if req.uri().path().starts_with("/v1/ai/jobs/") {
        return Ok(state.ai.handle(req).await);
    }
    if req.method() == Method::GET && req.uri().path() == "/v1/health" {
        let ai = state.ai.health();
        let mut capabilities = vec!["companion_diarization_v1"];
        if ai.available {
            capabilities.push("companion_ai_v1");
        }
        return Ok(match companion_readiness(&state.factory).await {
            Ok(s) => response(
                StatusCode::OK,
                serde_json::json!({
                    "version":1,
                    "desktop_version":env!("CARGO_PKG_VERSION"),
                    "engine":s.transcription.engine,
                    "model":match s.transcription.engine {
                        sbobino_domain::TranscriptionEngine::WhisperCpp=>s.transcription.model.ggml_filename(),
                        sbobino_domain::TranscriptionEngine::ParakeetCpp=>s.transcription.parakeet_model.gguf_filename()
                    },
                    // This is a protocol capability, independent of whether
                    // the optional pyannote model has been downloaded yet.
                    // Advertising it here lets an iPhone send an explicit
                    // per-job choice. If `true` cannot be fulfilled, the
                    // desktop still persists the ASR transcript and returns
                    // failed diarization metadata for that job.
                    "capabilities": capabilities,
                    "ai": ai,
                    "asr_available": true
                }),
            ),
            Err(message) => response(
                if ai.available {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                },
                serde_json::json!({"version":1,"desktop_version":env!("CARGO_PKG_VERSION"),"message":message,"asr_available":false,"capabilities":capabilities,"ai":ai}),
            ),
        });
    }
    let Some(id) = req
        .uri()
        .path()
        .strip_prefix("/v1/jobs/")
        .filter(|s| uuid::Uuid::parse_str(s).is_ok())
        .map(str::to_string)
    else {
        return Ok(response(
            StatusCode::NOT_FOUND,
            serde_json::json!({"message":"Richiesta non valida."}),
        ));
    };
    if req.method() == Method::GET || req.method() == Method::DELETE {
        if req.method() == Method::DELETE {
            if let Some(job) = state.jobs.lock().unwrap().get_mut(&id) {
                if !terminal(&job.snapshot.status) {
                    job.cancel.cancel();
                }
            }
        }
        return Ok(match snapshot(&state, &id) {
            Some(job) => response(StatusCode::OK, serde_json::to_value(job).unwrap()),
            None => response(
                StatusCode::NOT_FOUND,
                serde_json::json!({"message":"Trascrizione non trovata."}),
            ),
        });
    }
    if req.method() != Method::PUT {
        return Ok(response(
            StatusCode::METHOD_NOT_ALLOWED,
            serde_json::json!({"message":"Metodo non disponibile."}),
        ));
    }
    let options: HashMap<String, String> =
        url::form_urlencoded::parse(req.uri().query().unwrap_or("").as_bytes())
            .into_owned()
            .collect();
    let diarization_enabled = match options.get("diarization").map(String::as_str) {
        None => None,
        Some("true") => Some(true),
        Some("false") => Some(false),
        Some(_) => {
            return Ok(response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"message":"Il parametro diarization deve essere true o false."}),
            ));
        }
    };
    let extension = options.get("extension").map(String::as_str).unwrap_or("");
    if ![
        "mp3", "wav", "m4a", "aac", "aiff", "aif", "caf", "flac", "ogg", "mp4", "mov", "m4v",
    ]
    .contains(&extension)
    {
        return Ok(response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"message":"Formato audio/video non supportato."}),
        ));
    }
    let cancel = state.stop.child_token();
    {
        let mut jobs = state.jobs.lock().unwrap();
        if let Some(job) = jobs.get(&id) {
            return Ok(response(
                StatusCode::OK,
                serde_json::to_value(&job.snapshot).unwrap(),
            ));
        }
        if jobs.len() >= MAX_JOBS {
            // Completed results already live in the desktop archive.
            if let Some(old) = jobs
                .iter()
                .find(|(_, j)| terminal(&j.snapshot.status))
                .map(|(id, _)| id.clone())
            {
                jobs.remove(&old);
            } else {
                return Ok(response(
                    StatusCode::TOO_MANY_REQUESTS,
                    serde_json::json!({"message":"Attendi la fine delle trascrizioni in corso."}),
                ));
            }
        }
        jobs.insert(
            id.clone(),
            Job {
                cancel: cancel.clone(),
                snapshot: JobSnapshot {
                    id: id.clone(),
                    status: "uploading".into(),
                    text: String::new(),
                    message: "Ricevo il file…".into(),
                    percentage: 0,
                    artifact_id: None,
                    duration: None,
                    engine: None,
                    model: None,
                    timeline_v2: None,
                    diarization_status: None,
                    diarization_error: None,
                },
            },
        );
    }
    let upload = async {
        let directory = tempfile::Builder::new()
            .prefix("sbobino-companion-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let path = directory.path().join(format!("audio.{extension}"));
        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(|e| e.to_string())?;
        let mut bytes = 0u64;
        while let Some(frame) =
            tokio::time::timeout(Duration::from_secs(30), req.body_mut().frame())
                .await
                .map_err(|_| "Upload timeout")?
        {
            let frame = frame.map_err(|e| e.to_string())?;
            if let Some(data) = frame.data_ref() {
                bytes = bytes
                    .checked_add(data.len() as u64)
                    .ok_or("File troppo grande")?;
                if bytes > MAX_UPLOAD {
                    return Err("File troppo grande (massimo 2 GB)".to_string());
                }
                file.write_all(data).await.map_err(|e| e.to_string())?;
            }
        }
        if bytes == 0 {
            return Err("File vuoto".to_string());
        }
        file.flush().await.map_err(|e| e.to_string())?;
        Ok((directory, path))
    };
    let received = tokio::select! {
        _ = cancel.cancelled() => Err("Invio annullato".into()),
        result = upload => result,
    };
    let (directory, path) = match received {
        Ok(value) => value,
        Err(message) => {
            let cancelled = cancel.is_cancelled();
            update(&state, &id, |j| {
                j.status = if cancelled { "cancelled" } else { "failed" }.into();
                j.message = if cancelled {
                    "Annullata".into()
                } else {
                    message
                };
            });
            return Ok(response(
                StatusCode::BAD_REQUEST,
                serde_json::to_value(snapshot(&state, &id)).unwrap(),
            ));
        }
    };
    update(&state, &id, |j| {
        j.status = "queued".into();
        j.message = "In attesa sul desktop…".into();
    });
    let title = options
        .get("title")
        .map(|s| s.chars().take(200).collect::<String>());
    let job_state = state.clone();
    let job_id = id.clone();
    let task = tokio::spawn(async move {
        let _directory = directory; // Retain audio until the desktop vault has saved it.
        let permit = tokio::select! { _ = cancel.cancelled() => None, p = job_state.gate.clone().acquire_owned() => p.ok() };
        let Some(_permit) = permit else {
            update(&job_state, &job_id, |j| {
                j.status = "cancelled".into();
                j.message = "Annullata".into();
            });
            return;
        };
        let result = async {
            let settings_before_build = job_state
                .factory
                .load_settings()
                .map_err(ApplicationError::Settings)?;
            let service = job_state
                .factory
                .build_service_with_diarization_override(diarization_enabled)
                .map_err(ApplicationError::Settings)?;
            let settings_after_build = job_state
                .factory
                .load_settings()
                .map_err(ApplicationError::Settings)?;
            if !companion_settings_match(&settings_before_build, &settings_after_build) {
                return Err(ApplicationError::Settings(
                    "Le impostazioni di trascrizione sono cambiate mentre preparavo il job. Riprova.".into(),
                ));
            }
            let s = settings_after_build.transcription;
            let progress_state = job_state.clone();
            let progress_id = job_id.clone();
            let delta_state = job_state.clone();
            let delta_id = job_id.clone();
            update(&job_state, &job_id, |j| {
                j.status = "running".into();
                j.message = "Trascrivo sul desktop…".into();
            });
            service
                .run_file_transcription(
                    RunTranscriptionRequest {
                        job_id: job_id.clone(),
                        input_path: path.to_string_lossy().into_owned(),
                        engine: s.engine,
                        language: s.language,
                        model: s.model,
                        parakeet_model: s.parakeet_model,
                        enable_ai: false,
                        whisper_options: s.whisper_options,
                        title,
                        parent_id: None,
                        source_origin: ArtifactSourceOrigin::Imported,
                        metadata: std::collections::BTreeMap::from([(
                            "companion_protocol".into(),
                            "1".into(),
                        )]),
                        source_fingerprint_json: None,
                        diarization_enabled,
                    },
                    Arc::new(move |p| {
                        update(&progress_state, &progress_id, |j| {
                            j.percentage = p.overall_percentage;
                            j.message = p.message;
                        })
                    }),
                    Arc::new(move |text| {
                        update(&delta_state, &delta_id, |j| {
                            if let Some(next) = text.strip_prefix(REPLACE) {
                                j.text = next.into();
                            } else {
                                j.text.push_str(&text);
                            }
                        })
                    }),
                    cancel.clone(),
                )
                .await
        }
        .await;
        match result {
            Ok(artifact) => {
                if cancel.is_cancelled() {
                    update(&job_state, &job_id, |j| {
                        j.status = "cancelled".into();
                        j.message="Annullata; eventuali risultati già salvati restano nell'archivio desktop.".into();
                    });
                    return;
                }
                // Repository save enriches audio metadata. Emit the durable artifact,
                // not the pre-save value returned by the transcription service.
                let artifact = match job_state.factory.artifact_service().get(&artifact.id).await {
                    Ok(Some(saved)) if saved.raw_transcript == artifact.raw_transcript => saved,
                    _ => {
                        update(&job_state, &job_id, |j| {
                            j.status = "failed".into();
                            j.message =
                                "Non riesco a verificare il risultato nell’archivio desktop."
                                    .into();
                        });
                        return;
                    }
                };
                if cancel.is_cancelled() {
                    update(&job_state, &job_id, |j| {
                        j.status = "cancelled".into();
                        j.message =
                            "Annullata; il risultato già salvato resta nell'archivio desktop."
                                .into();
                    });
                    return;
                }
                update(&job_state, &job_id, |j| {
                    j.status = "completed".into();
                    j.text = artifact.raw_transcript.clone();
                    j.percentage = 100;
                    j.message = "Trascrizione completata".into();
                    j.artifact_id = Some(artifact.id.clone());
                    j.duration = artifact.audio_duration_seconds;
                    j.engine = artifact.processing_engine.clone();
                    j.model = artifact.processing_model.clone();
                    j.timeline_v2 = artifact
                        .metadata
                        .get("timeline_v2")
                        .and_then(|raw| slim_timeline_v2(raw));
                    j.diarization_status =
                        artifact.metadata.get("speaker_diarization_status").cloned();
                    j.diarization_error =
                        artifact.metadata.get("speaker_diarization_error").cloned();
                });
                (job_state.completed)(artifact);
            }
            Err(ApplicationError::Cancelled) => update(&job_state, &job_id, |j| {
                j.status = "cancelled".into();
                j.message = "Annullata".into();
            }),
            Err(error) => {
                tracing::warn!(%job_id, %error,"companion transcription failed");
                update(&job_state, &job_id, |j| {
                    j.status = "failed".into();
                    j.message =
                        "Trascrizione desktop non riuscita. Controlla il runtime sul computer."
                            .into();
                });
            }
        }
    });
    {
        let mut tasks = state.tasks.lock().unwrap();
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }
    Ok(response(
        StatusCode::ACCEPTED,
        serde_json::to_value(snapshot(&state, &id)).unwrap(),
    ))
}

async fn companion_readiness(
    factory: &Arc<RuntimeTranscriptionFactory>,
) -> Result<AppSettings, String> {
    let factory = factory.clone();
    let (settings, health) = tokio::task::spawn_blocking(move || {
        Ok::<_, String>((
            factory.load_settings()?,
            factory.runtime_health_preflight()?,
        ))
    })
    .await
    .map_err(|_| "Verifica del desktop non disponibile.".to_string())?
    .map_err(|_| "Runtime o impostazioni desktop non disponibili.".to_string())?;
    if !health.ffmpeg_available {
        return Err(
            "FFmpeg non disponibile. Completa la configurazione di Sbobino Desktop.".into(),
        );
    }
    match settings.transcription.engine {
        sbobino_domain::TranscriptionEngine::WhisperCpp => {
            if !health.whisper_cli_available || !health.model_present {
                return Err("Whisper o il modello selezionato non sono pronti sul desktop.".into());
            }
        }
        sbobino_domain::TranscriptionEngine::ParakeetCpp => {
            if !health.parakeet_cli_available || !health.parakeet_model_present {
                return Err(
                    "Parakeet o il modello selezionato non sono pronti sul desktop.".into(),
                );
            }
            let cli = std::path::Path::new(&health.parakeet_cli_resolved)
                .canonicalize()
                .map_err(|_| "Runtime Parakeet non disponibile.".to_string())?;
            let directory = cli.parent().ok_or("Runtime Parakeet non disponibile.")?;
            let worker = directory.join(if cfg!(windows) {
                "parakeet-batch-json.exe"
            } else {
                "parakeet-batch-json"
            });
            let mut command = tokio::process::Command::new(&worker);
            command.arg("--help").kill_on_drop(true);
            #[cfg(target_os = "macos")]
            {
                let mut paths = vec![directory.to_path_buf(), directory.join("../lib")];
                if let Some(existing) = std::env::var_os("DYLD_LIBRARY_PATH") {
                    paths.extend(std::env::split_paths(&existing));
                }
                if let Ok(paths) = std::env::join_paths(paths) {
                    command.env("DYLD_LIBRARY_PATH", paths);
                }
            }
            let output = tokio::time::timeout(Duration::from_secs(3), command.output())
                .await
                .map_err(|_| {
                    "Il worker Parakeet non risponde. Aggiorna il runtime desktop.".to_string()
                })?
                .map_err(|_| {
                    "Worker Parakeet mancante. Completa la configurazione desktop.".to_string()
                })?;
            let help = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if !worker_supports_thread_cap(&help) {
                return Err("Il worker Parakeet è obsoleto o incompatibile. Aggiorna il runtime desktop prima di inviare l’audio.".into());
            }
        }
    }
    Ok(settings)
}

/// Keep the companion response focused on measured segment and speaker
/// metadata. The desktop artifact remains the source of truth for optional
/// per-word timings and those are intentionally omitted from this wire payload.
fn slim_timeline_v2(raw: &str) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let segments = value.get_mut("segments")?.as_array_mut()?;
    for segment in segments {
        if let Some(object) = segment.as_object_mut() {
            object.remove("words");
        }
    }
    serde_json::to_string(&value).ok()
}

fn worker_supports_thread_cap(help: &str) -> bool {
    help.contains("--threads") && help.contains("--manifest")
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    bool::from(left.ct_eq(right))
}

fn companion_settings_match(before: &AppSettings, after: &AppSettings) -> bool {
    let before_transcription = &before.transcription;
    let after_transcription = &after.transcription;
    before_transcription.engine == after_transcription.engine
        && before_transcription.model == after_transcription.model
        && before_transcription.parakeet_model == after_transcription.parakeet_model
        && before_transcription.language == after_transcription.language
        && before_transcription.compute_device == after_transcription.compute_device
        && match (
            serde_json::to_vec(&before_transcription.whisper_options),
            serde_json::to_vec(&after_transcription.whisper_options),
        ) {
            (Ok(before_options), Ok(after_options)) => before_options == after_options,
            _ => false,
        }
}

fn terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use std::sync::Arc;

    #[test]
    fn old_worker_is_not_mistaken_for_a_ready_runtime() {
        assert!(!worker_supports_thread_cap(
            "usage: --model --manifest [--lang]"
        ));
        assert!(!worker_supports_thread_cap("dyld: missing library"));
        assert!(worker_supports_thread_cap(
            "usage: --model --manifest [--lang] [--threads N]"
        ));
    }

    #[test]
    fn companion_timeline_removes_duplicate_word_timings() {
        let raw = r#"{"version":2,"segments":[{"text":"Ciao","start_seconds":0.0,"end_seconds":1.0,"speaker_id":"speaker_1","speaker_label":"Speaker 1","words":[{"text":"Ciao","start_seconds":0.0,"end_seconds":1.0}]}]}"#;
        let slim = slim_timeline_v2(raw).expect("timeline should be valid JSON");
        assert!(slim.contains("speaker_id"));
        assert!(!slim.contains("words"));
    }

    #[test]
    fn companion_timeline_rejects_malformed_payloads() {
        assert!(slim_timeline_v2("not-json").is_none());
        assert!(slim_timeline_v2(r#"{"version":2,"segments":{}}"#).is_none());
    }

    async fn trusted_client(pairing: &Pairing) -> Client {
        let certificate = base64::engine::general_purpose::STANDARD
            .decode(&pairing.certificate)
            .expect("pairing certificate should be base64 DER");
        Client::builder()
            .add_root_certificate(reqwest::Certificate::from_der(&certificate).unwrap())
            .no_proxy()
            .build()
            .unwrap()
    }

    async fn test_server_with_gate(permits: usize) -> (CompanionServer, Client, tempfile::TempDir) {
        std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let factory = Arc::new(
            RuntimeTranscriptionFactory::new_for_tests(temporary.path(), None)
                .expect("test runtime factory"),
        );
        let server = CompanionServer::start(
            factory,
            Arc::new(Semaphore::new(permits)),
            false,
            "test".into(),
            Arc::new(|_| {}),
        )
        .await
        .expect("companion server should start");
        let client = trusted_client(&server.pairing).await;
        (server, client, temporary)
    }

    async fn test_server() -> (CompanionServer, Client, tempfile::TempDir) {
        test_server_with_gate(1).await
    }
    #[tokio::test]
    async fn shutdown_drains_session_and_new_pairing_revokes_previous_token() {
        let (server, _client, temporary) = test_server().await;
        let token = server.pairing.token.clone();
        server.shutdown().await;
        drop(server);
        let factory =
            Arc::new(RuntimeTranscriptionFactory::new_for_tests(temporary.path(), None).unwrap());
        let next = CompanionServer::start(
            factory,
            Arc::new(Semaphore::new(1)),
            false,
            "test".into(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
        let client = trusted_client(&next.pairing).await;
        let response = client
            .get(format!("{}/v1/health", next.pairing.endpoint))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(next.ai_service().store_is_available_for_test());
        next.shutdown().await;
    }
    #[tokio::test]
    async fn ai_http_requires_pairing_and_reports_validation_without_asr_assets() {
        let (server, client, _temporary) = test_server().await;
        let url = format!(
            "{}/v1/ai/jobs/{}",
            server.pairing.endpoint,
            uuid::Uuid::new_v4()
        );
        assert_eq!(
            client.get(&url).send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let response = client
            .get(&url)
            .bearer_auth(&server.pairing.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            "job_not_found"
        );
        let response = client
            .put(&url)
            .bearer_auth(&server.pairing.token)
            .body("not-json")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = client
            .put(&url)
            .bearer_auth(&server.pairing.token)
            .body("x".repeat(524289))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let health = client
            .get(format!("{}/v1/health", server.pairing.endpoint))
            .bearer_auth(&server.pairing.token)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(health["ai"]["available"], false);
        assert_eq!(health["ai"]["limits"]["max_request_bytes"], 524288);
    }
    #[test]
    fn cancellation_and_terminal_states_are_distinct() {
        assert!(terminal("completed"));
        assert!(terminal("failed"));
        assert!(terminal("cancelled"));
        assert!(!terminal("uploading"));
        assert!(!terminal("queued"));
        assert!(!terminal("running"));
    }

    #[test]
    fn settings_guard_rejects_transcription_changes() {
        let before = AppSettings::default();
        let mut after = before.clone();
        assert!(companion_settings_match(&before, &after));

        after.transcription.language = sbobino_domain::LanguageCode::It;
        assert!(!companion_settings_match(&before, &after));
    }

    #[tokio::test]
    async fn tls_pairing_authenticates_and_rejects_invalid_credentials() {
        let (server, client, _temporary) = test_server().await;
        let health = format!("{}/v1/health", server.pairing.endpoint);

        let response = client.get(&health).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = client
            .get(&health)
            .bearer_auth("wrong-token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = client
            .get(&health)
            .bearer_auth(&server.pairing.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = response.json().await.unwrap();
        assert!(body["message"].as_str().is_some());
    }

    #[tokio::test]
    async fn tls_protocol_rejects_paths_extensions_and_empty_uploads() {
        let (server, client, _temporary) = test_server().await;
        let base = format!("{}/v1", server.pairing.endpoint);
        let auth = |request: reqwest::RequestBuilder| request.bearer_auth(&server.pairing.token);

        let response = auth(client.get(format!("{base}/unknown")))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let id = uuid::Uuid::new_v4();
        let response = auth(
            client
                .put(format!("{base}/jobs/{id}?extension=txt"))
                .body("not audio"),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = auth(
            client
                .put(format!("{base}/jobs/{id}?extension=mp3"))
                .body(Vec::<u8>::new()),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let snapshot: JobSnapshot = response.json().await.unwrap();
        assert_eq!(snapshot.status, "failed");
        assert_eq!(snapshot.message, "File vuoto");
    }

    #[tokio::test]
    async fn dropping_server_stops_accepting_new_connections() {
        let (server, client, _temporary) = test_server().await;
        let health = format!("{}/v1/health", server.pairing.endpoint);
        drop(server);

        let mut stopped = false;
        for _ in 0..20 {
            if client
                .get(&health)
                .timeout(Duration::from_millis(100))
                .send()
                .await
                .is_err()
            {
                stopped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            stopped,
            "companion listener should stop after its owner is dropped"
        );
    }

    #[tokio::test]
    async fn deleting_a_queued_job_cancels_before_transcription_starts() {
        let (server, client, _temporary) = test_server_with_gate(0).await;
        let base = format!("{}/v1", server.pairing.endpoint);
        let id = uuid::Uuid::new_v4();

        let response = client
            .put(format!("{base}/jobs/{id}?extension=mp3"))
            .bearer_auth(&server.pairing.token)
            .body(vec![0_u8; 8])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let response = client
            .delete(format!("{base}/jobs/{id}"))
            .bearer_auth(&server.pairing.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let mut status = String::new();
        for _ in 0..20 {
            let response = client
                .get(format!("{base}/jobs/{id}"))
                .bearer_auth(&server.pairing.token)
                .send()
                .await
                .unwrap();
            let snapshot: JobSnapshot = response.json().await.unwrap();
            status = snapshot.status;
            if status == "cancelled" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(status, "cancelled");
    }
}
