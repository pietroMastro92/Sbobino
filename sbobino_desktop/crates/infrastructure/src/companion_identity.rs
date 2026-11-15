//! Durable, encrypted pairing identity for the opt-in TLS companion.
//!
//! The version-1 pairing JSON on the wire is unchanged: this module only keeps
//! the credentials behind it stable across restarts until an explicit
//! revocation. Everything lives under the isolated `factory.data_dir()`, is
//! encrypted with the desktop master key, and is never logged. No discovery,
//! network probing or silent rotation happens here.
use crate::secure_storage::{decrypt_from_file, encrypt_to_file, SecureStorage};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub(super) const DIRECTORY: &str = "companion-pairing";
const RECORD_FILE: &str = "pairing.enc";
const LEASE_FILE: &str = "session.lock";
/// Stable AES-GCM label; changing it makes existing records unreadable.
const RECORD_LABEL: &str = "companion-pairing:v1";
const RECORD_VERSION: u8 = 1;
const MAX_RECORD_BYTES: u64 = 256 * 1024;
const MAX_CERTIFICATE_BYTES: usize = 64 * 1024;
const MAX_KEY_BYTES: usize = 16 * 1024;

pub(super) const LEASE_BUSY: &str =
    "La sessione di abbinamento precedente si sta ancora chiudendo. Riprova tra poco.";
pub(super) const STORAGE_UNAVAILABLE: &str =
    "L'archivio cifrato dell'abbinamento non è disponibile.";
pub(super) const RECORD_UNREADABLE: &str =
    "L'abbinamento salvato non è leggibile. Verifica o revoca l'abbinamento, poi riprova.";
pub(super) const RECORD_VERSION_UNSUPPORTED: &str =
    "L'abbinamento salvato usa una versione non supportata. Verifica o revoca l'abbinamento, poi riprova.";
pub(super) const RECORD_INVALID: &str =
    "L'abbinamento salvato non è valido. Verifica o revoca l'abbinamento, poi riprova.";
pub(super) const RECORD_WRITE_FAILED: &str = "Non riesco a salvare l'abbinamento cifrato.";
pub(super) const RECORD_WRITE_UNCONFIRMED: &str =
    "L'abbinamento è stato scritto e potrebbe essere già attivo, ma non posso confermare che sia salvato in modo duraturo. Riapri Sbobino Desktop e verifica l'abbinamento.";
pub(super) const RECORD_REVOKE_FAILED: &str = "Non riesco a revocare l'abbinamento salvato.";
pub(super) const RECORD_REVOKE_UNCONFIRMED: &str =
    "L'abbinamento è stato rimosso e potrebbe essere già revocato, ma non posso confermare che la rimozione sia salvata in modo duraturo. Riapri Sbobino Desktop e verifica l'abbinamento.";
pub(super) const CREDENTIALS_UNAVAILABLE: &str =
    "Non riesco a preparare le credenziali dell'abbinamento.";

/// How the companion advertises itself. The record keeps this so a restart
/// cannot silently move an existing pairing between loopback and LAN.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PairingMode {
    Loopback,
    Lan,
}

impl PairingMode {
    fn wire(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::Lan => "lan",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "loopback" => Some(Self::Loopback),
            "lan" => Some(Self::Lan),
            _ => None,
        }
    }
}

/// The credentials and endpoint facts that must stay stable for a paired
/// iPhone: pinned certificate, TLS key, bearer token, port and advertised
/// address.
pub(super) struct PairingIdentity {
    pub(super) port: u16,
    pub(super) mode: PairingMode,
    pub(super) advertised: IpAddr,
    pub(super) certificate_der: Vec<u8>,
    pub(super) private_key_pkcs8: Vec<u8>,
    pub(super) bearer_token: String,
}

impl PairingIdentity {
    pub(super) fn generate(
        mode: PairingMode,
        advertised: IpAddr,
        port: u16,
    ) -> Result<Self, String> {
        let certified = rcgen::generate_simple_self_signed(vec![
            advertised.to_string(),
            "localhost".into(),
            "127.0.0.1".into(),
        ])
        .map_err(|_| CREDENTIALS_UNAVAILABLE.to_string())?;
        Ok(Self {
            port,
            mode,
            advertised,
            certificate_der: certified.cert.der().as_ref().to_vec(),
            private_key_pkcs8: certified.key_pair.serialize_der(),
            bearer_token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        })
    }
}

/// Pure policy check: an existing pairing is only reusable for the same mode
/// and the same advertised address. A different address needs verification or
/// a fresh pairing; this slice does not track IP changes.
pub(super) fn pairing_policy_mismatch(
    record_mode: PairingMode,
    record_address: IpAddr,
    requested_mode: PairingMode,
    requested_address: IpAddr,
) -> Option<&'static str> {
    if record_mode != requested_mode {
        return Some(
            "L'abbinamento salvato appartiene a una modalità di rete diversa. Verifica o revoca l'abbinamento, poi riprova.",
        );
    }
    if record_address != requested_address {
        return Some(
            "L'indirizzo locale è diverso da quello dell'abbinamento salvato. Verifica la rete o revoca l'abbinamento, poi riprova.",
        );
    }
    None
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityRecord {
    version: u8,
    mode: String,
    advertised_address: String,
    port: u16,
    certificate_der_base64: String,
    private_key_pkcs8_base64: String,
    bearer_token: String,
}

impl From<&PairingIdentity> for IdentityRecord {
    fn from(identity: &PairingIdentity) -> Self {
        Self {
            version: RECORD_VERSION,
            mode: identity.mode.wire().to_string(),
            advertised_address: identity.advertised.to_string(),
            port: identity.port,
            certificate_der_base64: STANDARD.encode(&identity.certificate_der),
            private_key_pkcs8_base64: STANDARD.encode(&identity.private_key_pkcs8),
            bearer_token: identity.bearer_token.clone(),
        }
    }
}

impl IdentityRecord {
    fn into_identity(self) -> Result<PairingIdentity, String> {
        if self.version != RECORD_VERSION {
            return Err(RECORD_VERSION_UNSUPPORTED.to_string());
        }
        let mode = PairingMode::parse(&self.mode).ok_or_else(|| RECORD_INVALID.to_string())?;
        let advertised: IpAddr = self
            .advertised_address
            .parse()
            .map_err(|_| RECORD_INVALID.to_string())?;
        if self.port == 0 {
            return Err(RECORD_INVALID.to_string());
        }
        if self.bearer_token.len() != 64
            || !self
                .bearer_token
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(RECORD_INVALID.to_string());
        }
        let certificate_der = STANDARD
            .decode(self.certificate_der_base64.as_bytes())
            .map_err(|_| RECORD_INVALID.to_string())?;
        if certificate_der.is_empty()
            || certificate_der.len() > MAX_CERTIFICATE_BYTES
            || certificate_der.first() != Some(&0x30)
        {
            return Err(RECORD_INVALID.to_string());
        }
        let private_key_pkcs8 = STANDARD
            .decode(self.private_key_pkcs8_base64.as_bytes())
            .map_err(|_| RECORD_INVALID.to_string())?;
        if private_key_pkcs8.is_empty() || private_key_pkcs8.len() > MAX_KEY_BYTES {
            return Err(RECORD_INVALID.to_string());
        }
        Ok(PairingIdentity {
            port: self.port,
            mode,
            advertised,
            certificate_der,
            private_key_pkcs8,
            bearer_token: self.bearer_token,
        })
    }
}

/// Encrypted pairing record plus the OS lease that serializes listener starts
/// and explicit revocations for one profile.
pub(super) struct PairingStore {
    root: PathBuf,
    storage: SecureStorage,
    lease: Mutex<Option<File>>,
}

impl PairingStore {
    pub(super) fn open(data_dir: &Path) -> Result<Self, String> {
        let root = data_dir.join(DIRECTORY);
        std::fs::create_dir_all(&root).map_err(|_| STORAGE_UNAVAILABLE.to_string())?;
        restrict_directory(&root).map_err(|_| STORAGE_UNAVAILABLE.to_string())?;
        let lease = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(LEASE_FILE))
            .map_err(|_| STORAGE_UNAVAILABLE.to_string())?;
        restrict_file(&lease).map_err(|_| STORAGE_UNAVAILABLE.to_string())?;
        lease.try_lock().map_err(|_| LEASE_BUSY.to_string())?;
        let storage = match SecureStorage::load_or_create_with_fallback(data_dir) {
            Ok(storage) => storage,
            Err(_) => {
                Self::unlock_lease(&lease);
                return Err(STORAGE_UNAVAILABLE.to_string());
            }
        };
        Ok(Self {
            root,
            storage,
            lease: Mutex::new(Some(lease)),
        })
    }

    fn record_path(&self) -> PathBuf {
        self.root.join(RECORD_FILE)
    }

    /// Reads the persisted identity for the requested advertised policy. A
    /// missing record is a normal first start. Anything unreadable, corrupted
    /// or incompatible is a hard error and the file is left untouched.
    pub(super) fn load(
        &self,
        mode: PairingMode,
        advertised: IpAddr,
    ) -> Result<Option<PairingIdentity>, String> {
        let path = self.record_path();
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(RECORD_UNREADABLE.to_string()),
        };
        if metadata.len() == 0 || metadata.len() > MAX_RECORD_BYTES {
            return Err(RECORD_UNREADABLE.to_string());
        }
        let bytes = decrypt_from_file(&self.storage, RECORD_LABEL, &path)
            .map_err(|_| RECORD_UNREADABLE.to_string())?;
        let record: IdentityRecord =
            serde_json::from_slice(&bytes).map_err(|_| RECORD_UNREADABLE.to_string())?;
        let identity = record.into_identity()?;
        if let Some(message) =
            pairing_policy_mismatch(identity.mode, identity.advertised, mode, advertised)
        {
            return Err(message.to_string());
        }
        Ok(Some(identity))
    }

    /// Writes the record atomically (sibling temp file + rename) and durably
    /// (file and directory sync), like the AI recovery store. A directory sync
    /// failure is reported and never swallowed: the rename already happened,
    /// so the record may already be in effect, but its durability is not
    /// confirmed.
    pub(super) fn persist(&self, identity: &PairingIdentity) -> Result<(), String> {
        let bytes = serde_json::to_vec(&IdentityRecord::from(identity))
            .map_err(|_| RECORD_WRITE_FAILED.to_string())?;
        let path = self.record_path();
        encrypt_to_file(&self.storage, RECORD_LABEL, &path, &bytes).map_err(|error| {
            tracing::error!(%error, "companion pairing record write failed");
            RECORD_WRITE_FAILED.to_string()
        })?;
        restrict_file_path(&path).map_err(|_| RECORD_WRITE_FAILED.to_string())?;
        sync_directory(&self.root).map_err(|error| {
            tracing::error!(%error, "companion pairing record directory sync failed");
            RECORD_WRITE_UNCONFIRMED.to_string()
        })?;
        Ok(())
    }

    /// Removes only the encrypted pairing record. Jobs, audio, artifacts, AI
    /// state and the master key are untouched; the lease file stays so a
    /// revoke cannot race a new start through an unlinked lock. A directory
    /// sync failure is reported and never swallowed: the unlink already
    /// happened, so the record is gone, but the removal's durability is not
    /// confirmed.
    pub(super) fn revoke(&self) -> Result<(), String> {
        match std::fs::remove_file(self.record_path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RECORD_REVOKE_FAILED.to_string()),
        }
        sync_directory(&self.root).map_err(|error| {
            tracing::error!(%error, "companion pairing revocation directory sync failed");
            RECORD_REVOKE_UNCONFIRMED.to_string()
        })?;
        Ok(())
    }

    /// Releases the profile lease before dropping the store, so a restart after
    /// an awaited shutdown does not need an out-of-process delay.
    pub(super) fn release_lease(&self) {
        let mut lease = match self.lease.lock() {
            Ok(lease) => lease,
            // No code panics while holding this mutex, so a poisoned lock still
            // holds the real lease: releasing it is the whole point of this
            // call. Taking the guard keeps the release exactly as fail-closed as
            // the normal path.
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(file) = lease.as_ref() {
            Self::unlock_lease(file);
        }
        let _ = lease.take();
    }

    /// Releases the OS lease explicitly instead of relying on the implicit
    /// release performed when the descriptor is closed.
    ///
    /// An explicit unlock ends ownership independently of when the last
    /// descriptor for this open file description closes. It also makes the
    /// shutdown/restart boundary explicit. A failure is logged; dropping the
    /// descriptor still provides the OS close-time release as a fallback.
    fn unlock_lease(file: &File) {
        if let Err(error) = file.unlock() {
            tracing::error!(%error, "companion pairing lease release failed");
        }
    }
}

impl Drop for PairingStore {
    fn drop(&mut self) {
        // The store owns the lease for the whole time it exists, so a store that
        // is dropped without an explicit `release_lease` (the owner went away
        // while the accept task drained) must release it the same way here.
        match self.lease.get_mut() {
            Ok(lease) => {
                if let Some(file) = lease.as_ref() {
                    Self::unlock_lease(file);
                }
            }
            Err(poisoned) => {
                if let Some(file) = poisoned.into_inner().as_ref() {
                    Self::unlock_lease(file);
                }
            }
        }
    }
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(file: &File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_file(_file: &File) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_path(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_file_path(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(root: &Path) -> std::io::Result<()> {
    File::open(root)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_root: &Path) -> std::io::Result<()> {
    // Windows cannot open a directory as a file; the AI recovery store keeps
    // the same platform behaviour.
    Ok(())
}

#[cfg(test)]
mod lease_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn explicit_release_and_drop_unlock_even_with_a_duplicated_descriptor() {
        std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
        for explicit_release in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let store = PairingStore::open(temp.path()).unwrap();
            let descriptor = store
                .lease
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .try_clone()
                .unwrap();
            assert!(PairingStore::open(temp.path()).is_err());
            if explicit_release {
                store.release_lease();
                // The store itself is still alive, as after shutdown().await.
                let replacement = PairingStore::open(temp.path()).unwrap();
                drop(replacement);
                drop(store);
            } else {
                drop(store);
                let replacement = PairingStore::open(temp.path()).unwrap();
                drop(replacement);
            }
            drop(descriptor);
        }
    }

    #[test]
    fn storage_open_failure_releases_the_pairing_lease_for_a_retry() {
        std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
        let temp = tempfile::tempdir().unwrap();
        let blocker = temp.path().join(".sbobino-secure-storage");
        std::fs::write(&blocker, b"synthetic storage blocker").unwrap();
        for _ in 0..2 {
            let error = PairingStore::open(temp.path())
                .err()
                .expect("blocked storage");
            assert_eq!(error, STORAGE_UNAVAILABLE);
        }
        std::fs::remove_file(blocker).unwrap();
        let reopened = PairingStore::open(temp.path())
            .expect("the storage failure must not leave a busy lease");
        drop(reopened);
    }
}
