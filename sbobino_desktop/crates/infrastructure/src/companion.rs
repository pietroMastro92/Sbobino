//! Opt-in TLS companion. Reuses the desktop service and its heavy-work gate.
#[path = "companion_ai.rs"]
mod ai;
#[path = "companion_identity.rs"]
mod identity;
use crate::fluid_diarization_runtime::{
    companion_diarization_readiness, DiarizationReadiness, HostPlatform,
};
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
    collections::{BTreeMap, HashMap},
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
/// Reserved artifact metadata key. Only a completed diarization run of the
/// current job writes it.
const SPEAKER_TURNS_METADATA_KEY: &str = "speaker_turns_v1";
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeakerTurnWire {
    pub speaker_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_label: Option<String>,
    /// Measured seconds, exactly as the artifact metadata wrote them. `f64`
    /// keeps the JSON number textual instead of widening an `f32` into
    /// `1.399999976158142` on the wire.
    pub start_seconds: f64,
    pub end_seconds: f64,
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
    /// Measured diarizer turns, additive and optional. Absent on
    /// failure/skip, for older desktops and for snapshots that never reached
    /// `completed`; a client must treat absence as "reconstruct locally".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_turns: Option<Vec<SpeakerTurnWire>>,
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
    /// Test-only observation of the accept-task drain. It exists so a test can
    /// prove the real lifecycle ordering (lease versus drain) without sleeping.
    #[cfg(test)]
    drain: DrainSignals,
}

/// Test-only lifecycle signals, fired exactly when the accept task starts and
/// finishes draining its connections and jobs.
#[cfg(test)]
#[derive(Default)]
struct DrainSignals {
    started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    finished: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[cfg(test)]
impl ServerState {
    /// Installs fresh drain signals and returns their receivers.
    fn watch_drain(
        &self,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let (finished, finished_rx) = tokio::sync::oneshot::channel();
        *self.drain.started.lock().unwrap() = Some(started);
        *self.drain.finished.lock().unwrap() = Some(finished);
        (started_rx, finished_rx)
    }
}

#[cfg(test)]
fn signal_drain(slot: &Mutex<Option<tokio::sync::oneshot::Sender<()>>>) {
    let sender = slot.lock().ok().and_then(|mut slot| slot.take());
    if let Some(sender) = sender {
        let _ = sender.send(());
    }
}

pub struct CompanionServer {
    pub pairing: Pairing,
    ai: Arc<AiService>,
    stop: CancellationToken,
    accept_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Holds the encrypted pairing record and the OS profile lease. The accept
    /// task keeps its own reference for the whole drain, so the lease outlives
    /// a `drop` without `shutdown`.
    identity: Arc<identity::PairingStore>,
    /// Test-only handle to the accept-task state.
    #[cfg(test)]
    state: Arc<ServerState>,
}

impl Drop for CompanionServer {
    fn drop(&mut self) {
        // Stop the listener, but do not touch the profile lease here: the
        // accept task owns its own reference to the pairing store, so a revoke
        // or a second start stays refused until that task has drained its
        // connections and jobs and that reference is dropped.
        self.stop.cancel();
    }
}

impl CompanionServer {
    /// Stops the listener, waits for the accept task to drain, and only then
    /// releases the profile lease. Concurrent calls are serialized on the task
    /// slot, so a second call never frees the lease while the drain is still
    /// running; a caller cancelled mid-drain leaves the handle in the slot, so
    /// the next call joins the same drain instead of detaching the task.
    pub async fn shutdown(&self) {
        self.stop.cancel();
        let mut slot = self.accept_task.lock().await;
        // The handle stays owned by the slot while it is awaited: if this
        // future is cancelled mid-drain, the task is not detached and a later
        // caller resumes the very same wait instead of skipping it.
        if let Some(task) = slot.as_mut() {
            let _ = task.await;
        }
        // Only a finished task is cleared, so the slot never looks drained
        // while the accept task is still alive.
        let _ = slot.take();
        self.ai.shutdown().await;
        // The listener is closed and the drain is complete: a revoke or a new
        // start may now take the profile lease even though this object still
        // exists. Suspending the companion never deletes the record.
        self.identity.release_lease();
    }
    pub fn ai_service(&self) -> Arc<AiService> {
        self.ai.clone()
    }

    /// Test-only handle used to inject a synthetic drain-blocking task and to
    /// observe the accept-task drain without sleeping.
    #[cfg(test)]
    fn test_state(&self) -> Arc<ServerState> {
        Arc::clone(&self.state)
    }

    pub async fn start(
        factory: Arc<RuntimeTranscriptionFactory>,
        gate: Arc<Semaphore>,
        lan: bool,
        desktop_version: String,
        completed: Arc<dyn Fn(TranscriptArtifact) + Send + Sync>,
    ) -> Result<Self, String> {
        let mode = if lan {
            identity::PairingMode::Lan
        } else {
            identity::PairingMode::Loopback
        };
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
        // One lease per profile: it serializes listener starts and explicit
        // revocations, and it is released only after shutdown or drop.
        let store = identity::PairingStore::open(factory.data_dir())?;
        let (listener, identity) = match store.load(mode, advertised_ip)? {
            Some(existing) => {
                let listener = TcpListener::bind((bind_ip, existing.port)).await.map_err(|_| {
                    format!(
                        "La porta {} dell'abbinamento non è disponibile. Libera la porta oppure revoca l'abbinamento per crearne uno nuovo.",
                        existing.port
                    )
                })?;
                (listener, existing)
            }
            None => {
                // First start: bind ephemeral, generate the identity, persist the
                // bound port, and only then start accepting connections.
                let listener = TcpListener::bind((bind_ip, 0)).await.map_err(|_| {
                    "Non riesco ad aprire il collegamento sul computer.".to_string()
                })?;
                let port = listener
                    .local_addr()
                    .map_err(|_| "Non riesco ad aprire il collegamento sul computer.".to_string())?
                    .port();
                let identity = identity::PairingIdentity::generate(mode, advertised_ip, port)?;
                store.persist(&identity)?;
                (listener, identity)
            }
        };
        let port = identity.port;
        let der = rustls::pki_types::CertificateDer::from(identity.certificate_der.clone());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|_| identity::CREDENTIALS_UNAVAILABLE.to_string())?
            .with_no_client_auth()
            .with_single_cert(
                vec![der.clone()],
                PrivatePkcs8KeyDer::from(identity.private_key_pkcs8.clone()).into(),
            )
            .map_err(|_| {
                "Le credenziali dell'abbinamento salvato non sono coerenti. Verifica o revoca l'abbinamento, poi riprova."
                    .to_string()
            })?;
        let token = identity.bearer_token.clone();
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
            #[cfg(test)]
            drain: DrainSignals::default(),
        });
        let store = Arc::new(store);
        let accept_store = Arc::clone(&store);
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let connections = Arc::new(Semaphore::new(8));
        let accept_state = Arc::clone(&state);
        let accept_task = tokio::spawn(async move {
            let mut connections_tasks = tokio::task::JoinSet::new();
            loop {
                while connections_tasks.try_join_next().is_some() {}
                let accepted = tokio::select! {
                    _ = accept_state.stop.cancelled() => break,
                    value = listener.accept() => value,
                };
                let Ok((stream, _)) = accepted else { break };
                let Ok(permit) = connections.clone().try_acquire_owned() else {
                    continue;
                };
                let acceptor = acceptor.clone();
                let state = accept_state.clone();
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
            for job in accept_state.jobs.lock().unwrap().values() {
                job.cancel.cancel();
            }
            #[cfg(test)]
            signal_drain(&accept_state.drain.started);
            let tasks = std::mem::take(&mut *accept_state.tasks.lock().unwrap());
            for task in tasks {
                let _ = task.await;
            }
            // This task holds its own reference to the store for the whole
            // drain, so the OS lease cannot be handed to a revoke or to a second
            // start while a connection or job task is still running. The lease
            // itself is released by an awaited `shutdown` (explicit `LOCK_UN`)
            // or, if the owner went away, by the store's `Drop` once this last
            // reference is dropped here.
            drop(accept_store);
            #[cfg(test)]
            signal_drain(&accept_state.drain.finished);
        });
        Ok(Self {
            accept_task: tokio::sync::Mutex::new(Some(accept_task)),
            ai,
            identity: store,
            #[cfg(test)]
            state,
            pairing: Pairing {
                version: 1,
                endpoint: format!("https://{advertised_ip}:{port}"),
                token,
                certificate: STANDARD.encode(identity.certificate_der.as_slice()),
                desktop_name: "Sbobino Desktop".into(),
                desktop_version,
            },
            stop,
        })
    }
}

/// Backend hook for the future Desktop revocation command. It acquires the
/// same profile lease as a server start, so a live companion is never
/// invalidated mid-session and a revoke can never race a start. Only the
/// encrypted pairing record is removed: jobs, audio, artifacts, AI state and
/// the master key are untouched. A later start generates new credentials.
pub fn revoke_persisted_pairing(data_dir: &std::path::Path) -> Result<(), String> {
    let store = identity::PairingStore::open(data_dir)?;
    store.revoke()
}

/// Read-only observation of the saved pairing record used by the desktop
/// status/reporting path.
///
/// The record is never opened or decrypted here: presence only means the
/// encrypted file exists for this profile. It deliberately does not promise
/// that the saved credentials are usable — a corrupted record, a master key
/// mismatch or a changed advertised address are still detected only by a real
/// `start`, which fails closed. This helper never creates the directory, the
/// lease, the record or any Keychain entry, and it never starts the listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistedPairingState {
    /// No saved pairing for this profile (also the normal first start).
    Absent,
    /// A regular `companion-pairing/pairing.enc` entry exists. Not a validity check.
    Present,
    /// The entry could not be inspected, or the profile layout is not the one
    /// this helper describes (for example `companion-pairing` is not a
    /// directory). The UI must not guess between paired and unpaired here.
    Unknown,
}

/// What one `metadata` observation conclusively says about a single path.
///
/// The four cases exist because "the path is missing" and "the path cannot be
/// inspected" are different answers for the status UI, and the platforms report
/// them differently: a child lookup below a non-directory component is
/// `ENOTDIR`/`NotADirectory` on Unix but `ERROR_PATH_NOT_FOUND`/`NotFound` on
/// Windows (see rust-lang/rust `library/std/src/sys/io/error/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairingEntryKind {
    Directory,
    RegularFile,
    /// Conclusively not there.
    Missing,
    /// Not a directory, not a regular file, unreadable or otherwise inconclusive.
    Uninspectable,
}

fn pairing_entry_kind(path: &std::path::Path) -> PairingEntryKind {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => PairingEntryKind::Directory,
        Ok(metadata) if metadata.is_file() => PairingEntryKind::RegularFile,
        Ok(_) => PairingEntryKind::Uninspectable,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PairingEntryKind::Missing,
        Err(_) => PairingEntryKind::Uninspectable,
    }
}

/// Pure decision table for the status answer, kept separate from the I/O so the
/// cross-platform cases can be exercised without a Windows runtime.
///
/// `record` is only meaningful when `profile_directory` is a `Directory`: a
/// missing profile directory cannot hold a saved pairing at all.
fn classify_persisted_pairing_state(
    profile_directory: PairingEntryKind,
    record: PairingEntryKind,
) -> PersistedPairingState {
    match profile_directory {
        PairingEntryKind::Directory => match record {
            PairingEntryKind::RegularFile => PersistedPairingState::Present,
            // A confirmed profile directory with no record has nothing saved.
            PairingEntryKind::Missing => PersistedPairingState::Absent,
            // Neither a regular file nor conclusive: never guess.
            PairingEntryKind::Directory | PairingEntryKind::Uninspectable => {
                PersistedPairingState::Unknown
            }
        },
        // The profile store was never created: the normal first start.
        PairingEntryKind::Missing => PersistedPairingState::Absent,
        // `companion-pairing` is not a directory, or cannot be inspected, so the
        // presence of a record cannot be established. This is the case Windows
        // reports as `NotFound` for the child lookup, which must not be read as
        // "unpaired".
        PairingEntryKind::RegularFile | PairingEntryKind::Uninspectable => {
            PersistedPairingState::Unknown
        }
    }
}

pub fn persisted_pairing_state(data_dir: &std::path::Path) -> PersistedPairingState {
    // A `data_dir` that exists but is not a directory makes every observation
    // below unreliable (on Windows the child lookup degrades to `NotFound`).
    if matches!(
        pairing_entry_kind(data_dir),
        PairingEntryKind::RegularFile | PairingEntryKind::Uninspectable
    ) {
        return PersistedPairingState::Unknown;
    }
    let profile_directory = data_dir.join(identity::DIRECTORY);
    classify_persisted_pairing_state(
        pairing_entry_kind(&profile_directory),
        // Probing the record below a non-directory is a harmless failed stat; the
        // profile directory already decides the answer in that case.
        pairing_entry_kind(&profile_directory.join("pairing.enc")),
    )
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
        return Ok(match companion_probe(&state.factory).await {
            Ok(probe) => {
                let failure = companion_runtime_failure(&probe).await;
                let mut body = serde_json::json!({
                    "version":1,
                    "desktop_version":env!("CARGO_PKG_VERSION"),
                    // A protocol capability, not a readiness claim: it stays
                    // static even when `readiness` says this desktop cannot
                    // serve diarization right now.
                    "capabilities": capabilities,
                    "ai": ai,
                    // Additive: the protocol capability above stays static, so
                    // the iPhone learns whether an explicit per-job
                    // `diarization=true` can actually be served right now.
                    "readiness": readiness_payload(&probe.diarization),
                });
                let status = match failure {
                    Some(message) => {
                        body["message"] = serde_json::json!(message);
                        body["asr_available"] = serde_json::json!(false);
                        if ai.available {
                            StatusCode::OK
                        } else {
                            StatusCode::SERVICE_UNAVAILABLE
                        }
                    }
                    None => {
                        body["engine"] = serde_json::json!(probe.settings.transcription.engine);
                        body["model"] =
                            serde_json::json!(match probe.settings.transcription.engine {
                                sbobino_domain::TranscriptionEngine::WhisperCpp =>
                                    probe.settings.transcription.model.ggml_filename(),
                                sbobino_domain::TranscriptionEngine::ParakeetCpp =>
                                    probe.settings.transcription.parakeet_model.gguf_filename(),
                            });
                        body["asr_available"] = serde_json::json!(true);
                        StatusCode::OK
                    }
                };
                response(status, body)
            }
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
    // Admission guard: `diarization=true` (explicit, or inherited from the
    // desktop default) must not accept up to 2 GB of audio that this desktop
    // cannot diarize. The check runs after auth/query/extension validation and
    // before the job snapshot exists or the request body is read, so a
    // rejected body is not consumed or persisted and leaves no job behind.
    // Bytes may already be in flight with an eager HTTP client; the iOS
    // health preflight is what prevents initiating the upload altogether.
    //
    // `diarization=false` keeps the existing ASR-only path untouched. An
    // unreadable preflight is left to the downstream job error instead of
    // inventing a readiness verdict. A retry of a job this server already
    // accepted stays an idempotent snapshot response and is never rejected
    // retroactively.
    if !state.jobs.lock().unwrap().contains_key(&id) && diarization_enabled != Some(false) {
        let probe = companion_probe(&state.factory).await.ok();
        let effective = diarization_enabled.unwrap_or_else(|| {
            probe
                .as_ref()
                .is_some_and(|probe| probe.settings.transcription.speaker_diarization.enabled)
        });
        if effective {
            if let Some(probe) = probe.filter(|probe| !probe.diarization.ready) {
                // The preflight awaited blocking work. A concurrent retry may
                // have accepted this id while it ran: preserve idempotence at
                // the point where we decide to reject the request.
                let jobs = state.jobs.lock().unwrap();
                if let Some(job) = jobs.get(&id) {
                    return Ok(response(
                        StatusCode::OK,
                        serde_json::to_value(&job.snapshot).unwrap(),
                    ));
                }
                return Ok(response(
                    StatusCode::CONFLICT,
                    diarization_unavailable_body(&probe.diarization),
                ));
            }
        }
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
                    speaker_turns: None,
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
                    j.speaker_turns =
                        measured_speaker_turns(&artifact.metadata, j.diarization_status.as_deref());
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

/// Settings, preflight health and derived readiness, computed once per route.
struct CompanionProbe {
    settings: AppSettings,
    health: crate::RuntimeHealth,
    diarization: DiarizationReadiness,
}

/// Loads settings and the preflight runtime health exactly once and derives
/// the per-job diarization readiness from that same snapshot.
///
/// This path never builds a diarizer, starts Python, runs `otool`, downloads
/// or touches the network. The readiness decision is pure and reuses the
/// health that `/v1/health` already computed.
async fn companion_probe(
    factory: &Arc<RuntimeTranscriptionFactory>,
) -> Result<CompanionProbe, String> {
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
    let diarization = companion_diarization_readiness(
        settings.transcription.speaker_diarization.backend,
        &settings.transcription.speaker_diarization.device,
        &health.fluid_diarization,
        health.pyannote.ready,
        &health.pyannote.reason_code,
        &HostPlatform::current(),
    );
    Ok(CompanionProbe {
        settings,
        health,
        diarization,
    })
}

fn readiness_payload(readiness: &DiarizationReadiness) -> serde_json::Value {
    serde_json::json!({
        "diarization": {
            "ready": readiness.ready,
            "reason_code": readiness.reason_code,
            "message": readiness.message,
        }
    })
}

/// `409` body for an admission rejection. `reason_code` is the stable
/// admission machine code; the specific cause lives in the readiness block
/// and in the static human `message`.
fn diarization_unavailable_body(readiness: &DiarizationReadiness) -> serde_json::Value {
    serde_json::json!({
        "message": readiness.message,
        "reason_code": "diarization_unavailable",
        "readiness": readiness_payload(readiness),
    })
}

/// The existing `/v1/health` ASR gate, unchanged in checks, order and
/// messages. `None` means the desktop can accept an ASR job.
async fn companion_runtime_failure(probe: &CompanionProbe) -> Option<String> {
    let health = &probe.health;
    let settings = &probe.settings;
    if !health.ffmpeg_available {
        return Some(
            "FFmpeg non disponibile. Completa la configurazione di Sbobino Desktop.".into(),
        );
    }
    match settings.transcription.engine {
        sbobino_domain::TranscriptionEngine::WhisperCpp => {
            if !health.whisper_cli_available || !health.model_present {
                return Some(
                    "Whisper o il modello selezionato non sono pronti sul desktop.".into(),
                );
            }
        }
        sbobino_domain::TranscriptionEngine::ParakeetCpp => {
            if !health.parakeet_cli_available || !health.parakeet_model_present {
                return Some(
                    "Parakeet o il modello selezionato non sono pronti sul desktop.".into(),
                );
            }
            let cli = match std::path::Path::new(&health.parakeet_cli_resolved).canonicalize() {
                Ok(cli) => cli,
                Err(_) => return Some("Runtime Parakeet non disponibile.".to_string()),
            };
            let Some(directory) = cli.parent() else {
                return Some("Runtime Parakeet non disponibile.".to_string());
            };
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
            let output = match tokio::time::timeout(Duration::from_secs(3), command.output()).await
            {
                Ok(Ok(output)) => output,
                Ok(Err(_)) => {
                    return Some(
                        "Worker Parakeet mancante. Completa la configurazione desktop.".to_string(),
                    )
                }
                Err(_) => {
                    return Some(
                        "Il worker Parakeet non risponde. Aggiorna il runtime desktop.".to_string(),
                    )
                }
            };
            let help = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if !worker_supports_thread_cap(&help) {
                return Some("Il worker Parakeet è obsoleto o incompatibile. Aggiorna il runtime desktop prima di inviare l’audio.".into());
            }
        }
    }
    None
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

/// Extracts the reserved measured-turn metadata for the companion snapshot.
///
/// The contract is explicit and lossless for valid data only: the field is
/// absent unless the job completed diarization, the metadata is a JSON array
/// and at least one record is structurally valid. Mixed invalid records are
/// discarded one by one; no interval is invented, sorted, merged or padded.
fn measured_speaker_turns(
    metadata: &BTreeMap<String, String>,
    diarization_status: Option<&str>,
) -> Option<Vec<SpeakerTurnWire>> {
    if diarization_status != Some("completed") {
        return None;
    }
    let raw = metadata.get(SPEAKER_TURNS_METADATA_KEY)?;
    let records: Vec<serde_json::Value> = serde_json::from_str(raw).ok()?;
    let turns = records
        .into_iter()
        .filter_map(|record| {
            let turn: SpeakerTurnWire = serde_json::from_value(record).ok()?;
            let speaker_id = turn.speaker_id.trim();
            if speaker_id.is_empty()
                || !turn.start_seconds.is_finite()
                || !turn.end_seconds.is_finite()
                || turn.start_seconds < 0.0
                || turn.end_seconds <= turn.start_seconds
            {
                return None;
            }
            Some(SpeakerTurnWire {
                speaker_id: speaker_id.to_string(),
                speaker_label: turn
                    .speaker_label
                    .as_deref()
                    .map(str::trim)
                    .filter(|label| !label.is_empty())
                    .map(str::to_string),
                start_seconds: turn.start_seconds,
                end_seconds: turn.end_seconds,
            })
        })
        .collect::<Vec<_>>();
    if turns.is_empty() {
        None
    } else {
        Some(turns)
    }
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

    fn test_snapshot(speaker_turns: Option<Vec<SpeakerTurnWire>>) -> JobSnapshot {
        JobSnapshot {
            id: "job".into(),
            status: "completed".into(),
            text: "Ciao".into(),
            message: "Trascrizione completata".into(),
            percentage: 100,
            artifact_id: Some("artifact".into()),
            duration: Some(2.0),
            engine: Some("whisper_cpp".into()),
            model: Some("ggml-base.bin".into()),
            timeline_v2: None,
            speaker_turns,
            diarization_status: Some("completed".into()),
            diarization_error: None,
        }
    }

    #[test]
    fn companion_snapshot_serializes_speaker_turns_additively() {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "speaker_diarization_status".to_string(),
            "completed".to_string(),
        );
        metadata.insert(
            SPEAKER_TURNS_METADATA_KEY.to_string(),
            r#"[{"speaker_id":"speaker_1","speaker_label":"Speaker 1","start_seconds":0.0,"end_seconds":1.4},{"speaker_id":"speaker_2","start_seconds":1.2,"end_seconds":2.0}]"#
                .to_string(),
        );
        metadata.insert(
            "timeline_v2".to_string(),
            r#"{"version":2,"segments":[{"text":"Ciao","start_seconds":0.0,"end_seconds":2.0,"speaker_id":"speaker_1","words":[{"text":"Ciao","start_seconds":0.0,"end_seconds":1.0}]}]}"#
                .to_string(),
        );

        let mut snapshot = test_snapshot(measured_speaker_turns(
            &metadata,
            metadata
                .get("speaker_diarization_status")
                .map(String::as_str),
        ));
        snapshot.timeline_v2 = metadata
            .get("timeline_v2")
            .and_then(|raw| slim_timeline_v2(raw));

        let wire = serde_json::to_value(&snapshot).expect("snapshot should serialize");
        assert_eq!(
            wire["speaker_turns"],
            serde_json::json!([
                {
                    "speaker_id": "speaker_1",
                    "speaker_label": "Speaker 1",
                    "start_seconds": 0.0,
                    "end_seconds": 1.4
                },
                {
                    "speaker_id": "speaker_2",
                    "start_seconds": 1.2,
                    "end_seconds": 2.0
                }
            ])
        );
        let timeline = wire["timeline_v2"].as_str().expect("slim timeline");
        assert!(!timeline.contains("words"));
        assert_eq!(wire["diarization_status"], "completed");

        // The field is omitted, not null, when there is no measurement.
        let without = serde_json::to_value(test_snapshot(None)).expect("snapshot serializes");
        assert!(without.get("speaker_turns").is_none());
    }

    #[test]
    fn legacy_companion_snapshot_decodes_without_speaker_turns() {
        let legacy = r#"{"id":"job","status":"completed","text":"Ciao","message":"m","percentage":100,"artifact_id":null,"duration":null,"engine":null,"model":null,"timeline_v2":null,"diarization_status":"completed","diarization_error":null}"#;
        let snapshot: JobSnapshot =
            serde_json::from_str(legacy).expect("legacy snapshot must keep decoding");
        assert!(snapshot.speaker_turns.is_none());
        assert_eq!(snapshot.diarization_status.as_deref(), Some("completed"));
    }

    #[test]
    fn companion_speaker_turns_require_completed_diarization() {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            SPEAKER_TURNS_METADATA_KEY.to_string(),
            r#"[{"speaker_id":"speaker_1","start_seconds":0.0,"end_seconds":1.0}]"#.to_string(),
        );

        for status in [None, Some("failed"), Some("skipped")] {
            assert!(measured_speaker_turns(&metadata, status).is_none());
        }
        assert!(measured_speaker_turns(&metadata, Some("completed")).is_some());
        assert!(measured_speaker_turns(&BTreeMap::new(), Some("completed")).is_none());
    }

    #[test]
    fn companion_speaker_turns_discard_mixed_invalid_records() {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            SPEAKER_TURNS_METADATA_KEY.to_string(),
            r#"[
                {"speaker_id":"speaker_1","speaker_label":"Speaker 1","start_seconds":0.0,"end_seconds":1.0},
                {"speaker_id":"   ","start_seconds":1.0,"end_seconds":2.0},
                {"speaker_id":"missing_end","start_seconds":1.0},
                {"speaker_id":"inverted","start_seconds":3.0,"end_seconds":2.0},
                {"speaker_id":"negative","start_seconds":-1.0,"end_seconds":1.0},
                {"speaker_id":"empty_span","start_seconds":2.0,"end_seconds":2.0},
                "not-an-object",
                {"speaker_id":"  speaker_2  ","speaker_label":"  Speaker 2  ","start_seconds":2.5,"end_seconds":4.0}
            ]"#
                .to_string(),
        );

        let turns = measured_speaker_turns(&metadata, Some("completed"))
            .expect("valid records must survive invalid neighbours");
        assert_eq!(
            turns,
            vec![
                SpeakerTurnWire {
                    speaker_id: "speaker_1".into(),
                    speaker_label: Some("Speaker 1".into()),
                    start_seconds: 0.0,
                    end_seconds: 1.0,
                },
                SpeakerTurnWire {
                    speaker_id: "speaker_2".into(),
                    speaker_label: Some("Speaker 2".into()),
                    start_seconds: 2.5,
                    end_seconds: 4.0,
                },
            ]
        );

        metadata.insert(
            SPEAKER_TURNS_METADATA_KEY.to_string(),
            r#"[{"speaker_id":"only_invalid","start_seconds":2.0,"end_seconds":1.0}]"#.to_string(),
        );
        assert!(measured_speaker_turns(&metadata, Some("completed")).is_none());
        metadata.insert(
            SPEAKER_TURNS_METADATA_KEY.to_string(),
            "not-json".to_string(),
        );
        assert!(measured_speaker_turns(&metadata, Some("completed")).is_none());
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
    async fn health_advertises_per_job_diarization_readiness_additively() {
        let (server, client, _temporary) = test_server().await;
        let response = client
            .get(format!("{}/v1/health", server.pairing.endpoint))
            .bearer_auth(&server.pairing.token)
            .send()
            .await
            .unwrap();
        // The test profile has no ffmpeg/ASR runtime, so the existing gate
        // still answers 503; readiness must be present anyway.
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = response.json().await.unwrap();

        // The capability stays a static protocol statement.
        assert_eq!(
            body["capabilities"],
            serde_json::json!(["companion_diarization_v1"])
        );
        assert_eq!(body["asr_available"], false);

        let readiness = &body["readiness"]["diarization"];
        assert_eq!(readiness["ready"], false);
        // The global toggle defaults to `false`; reporting `disabled`/ready
        // here would be the false positive this contract removes.
        assert_eq!(readiness["reason_code"], "no_backend");
        let message = readiness["message"].as_str().expect("static message");
        assert!(!message.is_empty());
        for forbidden in ["/", "\\", "Application Support", ".dylib", "libpython"] {
            assert!(
                !message.contains(forbidden),
                "readiness message leaks {forbidden}: {message}"
            );
        }
        server.shutdown().await;
    }

    #[tokio::test]
    async fn a_retried_accepted_job_is_not_retroactively_rejected_by_readiness() {
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

        // The same job id is retried with an explicit diarization request
        // while readiness is still false. The accepted job must win: no 409
        // and no phantom loss of the resumable snapshot.
        let response = client
            .put(format!("{base}/jobs/{id}?diarization=true&extension=mp3"))
            .bearer_auth(&server.pairing.token)
            .body(Vec::<u8>::new())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let snapshot: JobSnapshot = response.json().await.unwrap();
        assert_eq!(snapshot.id, id.to_string());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn explicit_diarization_is_rejected_before_the_body_and_leaves_no_job() {
        let (server, client, _temporary) = test_server().await;
        let base = format!("{}/v1", server.pairing.endpoint);
        let id = uuid::Uuid::new_v4();

        let response = client
            .put(format!("{base}/jobs/{id}?diarization=true&extension=wav"))
            .bearer_auth(&server.pairing.token)
            .body(Vec::<u8>::new())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["reason_code"], "diarization_unavailable");
        assert_eq!(body["readiness"]["diarization"]["ready"], false);
        assert_eq!(
            body["readiness"]["diarization"]["reason_code"],
            "no_backend"
        );
        assert!(body["message"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));

        // An empty body would normally create a failed "File vuoto" snapshot.
        // A 409 admission rejection must not reach that path.
        let response = client
            .get(format!("{base}/jobs/{id}"))
            .bearer_auth(&server.pairing.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn asr_only_and_legacy_uploads_keep_the_existing_validation_path() {
        let (server, client, _temporary) = test_server().await;
        let base = format!("{}/v1", server.pairing.endpoint);

        for query in ["diarization=false&extension=mp3", "extension=mp3"] {
            let id = uuid::Uuid::new_v4();
            let response = client
                .put(format!("{base}/jobs/{id}?{query}"))
                .bearer_auth(&server.pairing.token)
                .body(Vec::<u8>::new())
                .send()
                .await
                .unwrap();
            // No readiness gate for ASR-only or unset requests: the existing
            // empty-upload validation still owns the answer.
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
            let snapshot: JobSnapshot = response.json().await.unwrap();
            assert_eq!(snapshot.status, "failed");
            assert_eq!(snapshot.message, "File vuoto");
        }
        server.shutdown().await;
    }
    #[tokio::test]
    async fn shutdown_drains_session_and_restart_reuses_the_persisted_pairing() {
        // The profile is created through the persisting entry point so the
        // restart below rebinds a port the OS will not hand to another socket.
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let server = start_persisting_server(temporary.path())
            .await
            .expect("companion server should start");
        let pairing = server.pairing.clone();
        let token = server.pairing.token.clone();
        // A Tauri command awaits this future, so it must stay `Send` even
        // though it now holds the accept-task slot across the drain.
        fn require_send<T: Send>(_: T) {}
        require_send(server.shutdown());
        server.shutdown().await;
        drop(server);
        let next = start_persisting_server(temporary.path())
            .await
            .expect("the persisted pairing should restart");
        let client = trusted_client(&next.pairing).await;
        let response = client
            .get(format!("{}/v1/health", next.pairing.endpoint))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        // The confirmed product decision: a restart keeps the pairing until an
        // explicit revocation. Only revocation invalidates the old token.
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(next.pairing.endpoint, pairing.endpoint);
        assert_eq!(next.pairing.certificate, pairing.certificate);
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

    fn pairing_record(data_dir: &std::path::Path) -> std::path::PathBuf {
        data_dir.join("companion-pairing").join("pairing.enc")
    }

    /// Persists a loopback pairing on a free port below the default ephemeral
    /// ranges on macOS, Linux and Windows, and returns it.
    ///
    /// A restart test must be about the persisted identity, not about the OS
    /// keeping a freed ephemeral port free: a client socket can be handed a port
    /// the companion just released, and a plain bind then legitimately fails.
    /// The atomic sequence assigns distinct candidates within this test binary,
    /// including when a candidate is already occupied. An unrelated process can
    /// still occupy a port between the probe and the server's bind.
    fn persist_stable_loopback_pairing(data_dir: &std::path::Path) -> u16 {
        const FIRST: u16 = 20_000;
        const SPAN: u16 = 800;
        static NEXT_CANDIDATE: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
        for _ in 0..SPAN {
            let sequence = NEXT_CANDIDATE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let offset = (std::process::id() % SPAN as u32) as u16;
            let candidate = FIRST + (sequence.wrapping_add(offset) % SPAN);
            let Ok(probe) = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, candidate)) else {
                continue;
            };
            drop(probe);
            let store = identity::PairingStore::open(data_dir).expect("pairing store");
            store
                .persist(
                    &identity::PairingIdentity::generate(
                        identity::PairingMode::Loopback,
                        IpAddr::V4(Ipv4Addr::LOCALHOST),
                        candidate,
                    )
                    .expect("synthetic identity"),
                )
                .expect("synthetic pairing record");
            store.release_lease();
            drop(store);
            return candidate;
        }
        panic!("no free non-ephemeral loopback port in the reserved test range");
    }

    async fn start_persisting_server(
        data_dir: &std::path::Path,
    ) -> Result<CompanionServer, String> {
        // A fresh profile in these tests starts from a port the OS will not hand
        // to an unrelated socket, so restarts do not race ephemeral reassignment.
        if !pairing_record(data_dir).exists() {
            persist_stable_loopback_pairing(data_dir);
        }
        CompanionServer::start(
            Arc::new(
                RuntimeTranscriptionFactory::new_for_tests(data_dir, None)
                    .expect("test runtime factory"),
            ),
            Arc::new(Semaphore::new(1)),
            false,
            "test".into(),
            Arc::new(|_| {}),
        )
        .await
    }

    #[test]
    fn companion_server_stays_shareable_with_the_desktop_state() {
        // The Tauri state holds this type across async commands; the durable
        // identity and its lease must not make it thread-affine.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CompanionServer>();
    }

    #[tokio::test]
    async fn pairing_identity_survives_restart_even_with_a_new_factory() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let first = start_persisting_server(temporary.path())
            .await
            .expect("companion server should start");
        let original = first.pairing.clone();
        assert_eq!(original.version, 1);
        assert_eq!(original.token.len(), 64);
        assert!(original.token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        first.shutdown().await;
        drop(first);

        let restarted = start_persisting_server(temporary.path())
            .await
            .expect("the persisted pairing should restart");
        assert_eq!(restarted.pairing.endpoint, original.endpoint);
        assert_eq!(restarted.pairing.token, original.token);
        assert_eq!(restarted.pairing.certificate, original.certificate);

        // The iPhone keeps its pinned certificate and bearer token, so no new
        // pairing is required after a Desktop restart. The client trusts only
        // that certificate and never disables verification.
        let client = trusted_client(&original).await;
        let response = client
            .get(format!("{}/v1/health", restarted.pairing.endpoint))
            .bearer_auth(&original.token)
            .send()
            .await
            .expect("pinned client should still connect");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        restarted.shutdown().await;
    }

    #[tokio::test]
    async fn persisted_pairing_record_is_encrypted_and_private() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let server = start_persisting_server(temporary.path())
            .await
            .expect("companion server should start");
        let record = pairing_record(temporary.path());
        let bytes = std::fs::read(&record).expect("pairing record should exist");
        assert!(!bytes.is_empty());
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(&server.pairing.token));
        assert!(!text.contains(&server.pairing.certificate));
        assert!(!text.contains("bearer_token"));
        assert!(!text.contains("private_key_pkcs8_base64"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let record_mode = std::fs::metadata(&record).unwrap().permissions().mode() & 0o777;
            assert_eq!(record_mode, 0o600);
            let directory_mode = std::fs::metadata(record.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(directory_mode, 0o700);
        }
        server.shutdown().await;
    }

    #[tokio::test]
    async fn unreadable_pairing_record_fails_closed_without_rewriting() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let first = start_persisting_server(temporary.path())
            .await
            .expect("companion server should start");
        first.shutdown().await;
        drop(first);

        let record = pairing_record(temporary.path());
        std::fs::write(&record, b"not a pairing record").unwrap();
        let corrupted = std::fs::read(&record).unwrap();
        let error = start_persisting_server(temporary.path())
            .await
            .err()
            .expect("start must fail closed");
        assert!(error.contains("abbinamento"), "{error}");
        assert_eq!(std::fs::read(&record).unwrap(), corrupted);
    }

    #[tokio::test]
    async fn occupied_persisted_port_fails_without_rotating_identity() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        // The persisted port is owned for the whole test instead of re-binding
        // the ephemeral port that a previous listener happened to use: a freed
        // ephemeral port can be handed to any other socket in this process, and
        // binding it again is then a race against unrelated traffic rather than
        // a statement about the companion. Holding the listener makes the
        // conflict itself deterministic.
        let occupier =
            std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a port to occupy");
        let port = occupier.local_addr().expect("occupier address").port();
        let store = identity::PairingStore::open(temporary.path()).expect("pairing store");
        store
            .persist(
                &identity::PairingIdentity::generate(
                    identity::PairingMode::Loopback,
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    port,
                )
                .expect("synthetic identity"),
            )
            .expect("synthetic pairing record");
        store.release_lease();
        drop(store);

        let record = pairing_record(temporary.path());
        let before = std::fs::read(&record).expect("pairing record should exist");

        let error = start_persisting_server(temporary.path())
            .await
            .err()
            .expect("a busy persisted port must not fall back to a random port");
        assert!(error.contains(&port.to_string()), "{error}");
        assert_eq!(std::fs::read(&record).unwrap(), before);
        drop(occupier);
    }

    #[tokio::test]
    async fn pairing_lease_blocks_overlapping_start_and_revocation() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let active = start_persisting_server(temporary.path())
            .await
            .expect("companion server should start");
        let original = active.pairing.clone();

        assert!(start_persisting_server(temporary.path()).await.is_err());
        assert!(revoke_persisted_pairing(temporary.path()).is_err());
        assert_eq!(active.pairing.token, original.token);

        active.shutdown().await;
        drop(active);

        revoke_persisted_pairing(temporary.path()).expect("revocation after shutdown");
        assert!(!pairing_record(temporary.path()).exists());
        // The lease file stays in place so a revoke cannot race a new start.
        assert!(temporary
            .path()
            .join("companion-pairing")
            .join("session.lock")
            .exists());

        let fresh = start_persisting_server(temporary.path())
            .await
            .expect("a revoked profile starts fresh");
        assert_ne!(fresh.pairing.token, original.token);
        assert_ne!(fresh.pairing.certificate, original.certificate);

        let current = trusted_client(&fresh.pairing).await;
        let response = current
            .get(format!("{}/v1/health", fresh.pairing.endpoint))
            .bearer_auth(&original.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let stale = trusted_client(&original).await;
        let stale_attempt = stale
            .get(format!("{}/v1/health", fresh.pairing.endpoint))
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        assert!(
            stale_attempt.is_err(),
            "the revoked certificate must not be trusted"
        );
        fresh.shutdown().await;
    }

    #[test]
    fn advertised_policy_mismatch_is_reported_separately() {
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let synthetic = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
        assert!(identity::pairing_policy_mismatch(
            identity::PairingMode::Loopback,
            loopback,
            identity::PairingMode::Loopback,
            loopback
        )
        .is_none());
        let mode = identity::pairing_policy_mismatch(
            identity::PairingMode::Lan,
            synthetic,
            identity::PairingMode::Loopback,
            loopback,
        )
        .expect("a different mode must be reported");
        let address = identity::pairing_policy_mismatch(
            identity::PairingMode::Lan,
            synthetic,
            identity::PairingMode::Lan,
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 11)),
        )
        .expect("a different address must be reported");
        assert!(mode.contains("modalità"), "{mode}");
        assert!(address.contains("indirizzo"), "{address}");
        assert_ne!(mode, address);
    }

    #[tokio::test]
    async fn record_from_another_network_requires_explicit_verification() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let store = identity::PairingStore::open(temporary.path()).expect("pairing store");
        let synthetic = identity::PairingIdentity::generate(
            identity::PairingMode::Lan,
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            443,
        )
        .expect("synthetic identity");
        store.persist(&synthetic).expect("synthetic record");
        store.release_lease();
        drop(store);

        let record = pairing_record(temporary.path());
        let before = std::fs::read(&record).unwrap();
        let error = start_persisting_server(temporary.path())
            .await
            .err()
            .expect("a different advertised policy must fail");
        assert!(error.contains("modalità"), "{error}");
        assert_eq!(std::fs::read(&record).unwrap(), before);
    }

    /// A waker that never reschedules anything, used to poll a shutdown future
    /// synchronously and prove it cannot complete while the drain is blocked.
    struct NoopWake;

    impl std::task::Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    /// Pushes a synthetic task into the accept-task drain that only ends when
    /// the returned sender is used. No sleeping is involved: the accept task
    /// provably cannot exit while this task is blocked.
    fn block_drain(
        state: &Arc<ServerState>,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (started, finished) = state.watch_drain();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        state.tasks.lock().unwrap().push(tokio::spawn(async move {
            let _ = released.await;
        }));
        (started, finished, release)
    }

    #[tokio::test]
    async fn dropped_server_keeps_the_pairing_lease_until_the_accept_task_drains() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let server = start_persisting_server(temporary.path())
            .await
            .expect("companion server should start");
        let state = server.test_state();
        let (drain_started, drain_finished, release) = block_drain(&state);
        let record = pairing_record(temporary.path());
        let before = std::fs::read(&record).expect("pairing record should exist");

        // No `shutdown` and no `sleep`: `Drop` alone must not hand the profile
        // to a revoke or to a second start while a task is still draining.
        drop(server);
        drain_started
            .await
            .expect("the accept task should enter its drain");

        assert!(
            revoke_persisted_pairing(temporary.path()).is_err(),
            "revocation must stay refused while the accept task drains"
        );
        assert!(
            start_persisting_server(temporary.path()).await.is_err(),
            "a second start must stay refused while the accept task drains"
        );
        assert_eq!(std::fs::read(&record).unwrap(), before);

        release.send(()).expect("the drain is still blocked");
        drain_finished
            .await
            .expect("the accept task should finish its drain");

        revoke_persisted_pairing(temporary.path()).expect("revocation after the drain");
        assert!(!record.exists());
        let fresh = start_persisting_server(temporary.path())
            .await
            .expect("a drained profile starts fresh");
        fresh.shutdown().await;
        // The awaited shutdown released the lease even though `fresh` is still
        // alive, so an immediate restart reuses the persisted pairing.
        let again = start_persisting_server(temporary.path())
            .await
            .expect("an awaited shutdown should release the lease");
        assert_eq!(again.pairing.token, fresh.pairing.token);
        again.shutdown().await;
    }

    #[tokio::test]
    async fn concurrent_shutdowns_release_the_lease_only_after_the_drain() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let server = Arc::new(
            start_persisting_server(temporary.path())
                .await
                .expect("companion server should start"),
        );
        let state = server.test_state();
        let (drain_started, mut drain_finished, release) = block_drain(&state);

        let first = tokio::spawn({
            let server = Arc::clone(&server);
            async move { server.shutdown().await }
        });
        drain_started
            .await
            .expect("the first shutdown should enter the drain");

        // Release the drain from a separate task so the second shutdown can be
        // awaited on this task without deadlocking.
        let (go, released_on_go) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if released_on_go.await.is_ok() {
                let _ = release.send(());
            }
        });
        go.send(()).expect("release task should be alive");

        // A concurrent shutdown must join the same drain instead of returning
        // early and freeing the profile lease while the drain is still running.
        server.shutdown().await;
        first.await.expect("the first shutdown should finish");
        assert!(
            drain_finished.try_recv().is_ok(),
            "a concurrent shutdown must not return before the drain ends"
        );
        assert!(
            revoke_persisted_pairing(temporary.path()).is_ok(),
            "the lease is released once both shutdowns have drained"
        );
    }

    /// A caller cancelled while awaiting the drain must not detach the accept
    /// task: the server keeps owning the handle, so a later caller still waits
    /// for the real drain before the profile lease is released.
    #[tokio::test]
    async fn cancelled_shutdown_keeps_the_drain_owned_until_the_accept_task_finishes() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let server = Arc::new(
            start_persisting_server(temporary.path())
                .await
                .expect("companion server should start"),
        );
        let state = server.test_state();
        let (drain_started, mut drain_finished, release) = block_drain(&state);

        let first = tokio::spawn({
            let server = Arc::clone(&server);
            async move { server.shutdown().await }
        });
        drain_started
            .await
            .expect("the first shutdown should enter the drain");
        // Cancel the first caller while the accept task is still draining, and
        // wait until its future has really been dropped.
        first.abort();
        let _ = first.await;

        // The second caller is polled synchronously with a waker that never
        // fires. The runtime cannot complete anything behind our back, so a
        // bounded poll loop is deterministic: a shutdown that returns here has
        // skipped the drain.
        let waker = std::task::Waker::from(Arc::new(NoopWake));
        let mut context = std::task::Context::from_waker(&waker);
        let mut second = Box::pin(server.shutdown());
        for _ in 0..8 {
            assert!(
                matches!(
                    std::future::Future::poll(second.as_mut(), &mut context),
                    std::task::Poll::Pending
                ),
                "a shutdown must not return while the accept task drains"
            );
        }
        assert_eq!(
            revoke_persisted_pairing(temporary.path())
                .expect_err("revocation must stay refused while the accept task drains"),
            identity::LEASE_BUSY
        );
        assert_eq!(
            start_persisting_server(temporary.path())
                .await
                .err()
                .expect("a second start must stay refused while the accept task drains"),
            identity::LEASE_BUSY
        );

        release.send(()).expect("the drain is still blocked");
        second.await;
        assert!(
            drain_finished.try_recv().is_ok(),
            "the surviving shutdown must wait for the real drain"
        );
        // The server object is still alive: the completed shutdown released the
        // profile lease, so an explicit revocation and a fresh start work.
        revoke_persisted_pairing(temporary.path()).expect("revocation after the drain");
        let fresh = start_persisting_server(temporary.path())
            .await
            .expect("an awaited shutdown should release the lease");
        fresh.shutdown().await;
    }

    #[cfg(unix)]
    #[test]
    fn revocation_reports_a_directory_sync_failure_after_removing_the_record() {
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let store = identity::PairingStore::open(temporary.path()).expect("pairing store");
        let root = temporary.path().join(identity::DIRECTORY);
        // A profile directory that no longer exists cannot be fsynced.
        std::fs::remove_dir_all(&root).expect("profile directory");

        let error = store
            .revoke()
            .expect_err("an unconfirmed directory sync must be reported");
        assert!(error.contains("non posso confermare"), "{error}");
        // The unlink already happened before the sync failed: the message
        // describes exactly that partial outcome instead of claiming success.
        assert!(!pairing_record(temporary.path()).exists());
    }

    #[cfg(unix)]
    #[test]
    fn persist_reports_a_directory_sync_failure_after_the_rename() {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir().expect("temporary data directory");
        let store = identity::PairingStore::open(temporary.path()).expect("pairing store");
        let root = temporary.path().join(identity::DIRECTORY);
        let synthetic = identity::PairingIdentity::generate(
            identity::PairingMode::Loopback,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            4242,
        )
        .expect("synthetic identity");

        // Write and execute without read: the encrypted file can still be
        // renamed into place, but the directory cannot be opened to fsync it.
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o300))
            .expect("restrict directory");
        let exercised = std::fs::File::open(&root).is_err();
        let result = store.persist(&synthetic);
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("restore directory");

        if !exercised {
            // A privileged user or a filesystem that ignores mode bits cannot
            // exercise this ordering; the assertion below is the meaningful one.
            eprintln!("mode bits are not enforced here: directory sync was not exercised");
            return;
        }
        let error = result.expect_err("an unconfirmed directory sync must be reported");
        assert!(error.contains("non posso confermare"), "{error}");
        // The rename already happened, so the record exists even though its
        // durability could not be confirmed.
        assert!(pairing_record(temporary.path()).exists());
    }

    #[test]
    fn persisted_pairing_state_is_read_only_and_distinguishes_unreadable_profiles() {
        let empty = tempfile::tempdir().expect("temporary data directory");
        assert_eq!(
            persisted_pairing_state(empty.path()),
            PersistedPairingState::Absent
        );
        // Status reporting must never create the profile store just to answer.
        assert!(!empty.path().join(identity::DIRECTORY).exists());

        let paired = tempfile::tempdir().expect("temporary data directory");
        let root = paired.path().join(identity::DIRECTORY);
        std::fs::create_dir_all(&root).expect("pairing directory");
        // Synthetic bytes: presence is a metadata-only observation and the
        // helper must not need a readable or decodable record.
        std::fs::write(root.join("pairing.enc"), b"synthetic-bytes-not-decrypted")
            .expect("synthetic record");
        assert_eq!(
            persisted_pairing_state(paired.path()),
            PersistedPairingState::Present
        );

        let irregular = tempfile::tempdir().expect("temporary data directory");
        std::fs::create_dir_all(
            irregular
                .path()
                .join(identity::DIRECTORY)
                .join("pairing.enc"),
        )
        .expect("irregular record");
        assert_eq!(
            persisted_pairing_state(irregular.path()),
            PersistedPairingState::Unknown
        );

        let unreadable = tempfile::tempdir().expect("temporary data directory");
        std::fs::write(
            unreadable.path().join(identity::DIRECTORY),
            b"not-a-directory",
        )
        .expect("blocking file");
        assert_eq!(
            persisted_pairing_state(unreadable.path()),
            PersistedPairingState::Unknown
        );

        // A `data_dir` that exists but is not a directory cannot hold a profile.
        // Reading that as "unpaired" would be a guess, not an observation.
        let replaced_data_dir = tempfile::tempdir().expect("temporary data directory");
        let data_dir_file = replaced_data_dir.path().join("profile-dir-is-a-file");
        std::fs::write(&data_dir_file, b"not-a-directory").expect("blocking file");
        assert_eq!(
            persisted_pairing_state(&data_dir_file),
            PersistedPairingState::Unknown
        );
    }

    /// The layout evidence the status path can collect, regardless of platform.
    ///
    /// `RegularFile` profile directory + `Missing` record is exactly what the
    /// Windows runner observed before the fix: `metadata` on the child of a
    /// regular file returns `ERROR_PATH_NOT_FOUND`, which Rust maps to
    /// `NotFound`, so the old `NotFound => Absent` rule answered "unpaired" for
    /// a profile that cannot be inspected at all. Unix reports the same layout as
    /// `ENOTDIR` (`NotADirectory`), which is why only the Windows runner failed.
    #[test]
    fn persisted_pairing_classification_never_reads_an_uninspectable_profile_as_absent() {
        let cases = [
            (
                PairingEntryKind::Missing,
                PairingEntryKind::Missing,
                PersistedPairingState::Absent,
            ),
            (
                PairingEntryKind::Directory,
                PairingEntryKind::Missing,
                PersistedPairingState::Absent,
            ),
            (
                PairingEntryKind::Directory,
                PairingEntryKind::RegularFile,
                PersistedPairingState::Present,
            ),
            (
                PairingEntryKind::Directory,
                PairingEntryKind::Directory,
                PersistedPairingState::Unknown,
            ),
            (
                PairingEntryKind::Directory,
                PairingEntryKind::Uninspectable,
                PersistedPairingState::Unknown,
            ),
            (
                PairingEntryKind::RegularFile,
                PairingEntryKind::Missing,
                PersistedPairingState::Unknown,
            ),
            (
                PairingEntryKind::RegularFile,
                PairingEntryKind::RegularFile,
                PersistedPairingState::Unknown,
            ),
            (
                PairingEntryKind::Uninspectable,
                PairingEntryKind::Missing,
                PersistedPairingState::Unknown,
            ),
        ];
        for (profile_directory, record, expected) in cases {
            assert_eq!(
                classify_persisted_pairing_state(profile_directory, record),
                expected,
                "profile directory {profile_directory:?} with record {record:?}"
            );
        }
    }
}
