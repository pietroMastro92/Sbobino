use std::io::{BufReader, BufWriter, Read, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

use ring::{
    aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM},
    hkdf::{Salt, HKDF_SHA256},
    pbkdf2,
    rand::{SecureRandom, SystemRandom},
};

use sbobino_application::ApplicationError;

const KEYCHAIN_SERVICE_NAME: &str = "com.sbobino.desktop";
const MASTER_KEY_ACCOUNT: &str = "master_key_v1";
const LOCAL_FALLBACK_ENV: &str = "SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS";
const BACKUP_FILE_MAGIC: &[u8; 8] = b"SBOBBAK1";
const BACKUP_SALT_LEN: usize = 16;
const BACKUP_NONCE_PREFIX_LEN: usize = 8;
const BACKUP_CHUNK_SIZE: usize = 1024 * 1024;
const BACKUP_PBKDF2_ITERATIONS: u32 = 150_000;

#[derive(Debug, Clone)]
enum SecureStorageBackend {
    Keychain,
    LocalDir(PathBuf),
}

#[derive(Debug, Clone)]
pub struct SecureStorage {
    master_key: [u8; 32],
    backend: SecureStorageBackend,
}

impl SecureStorage {
    pub fn load_or_create() -> Result<Self, ApplicationError> {
        Self::load_or_create_internal(None)
    }

    pub fn load_or_create_with_fallback(root: &Path) -> Result<Self, ApplicationError> {
        Self::load_or_create_internal(Some(root))
    }

    fn load_or_create_internal(fallback_root: Option<&Path>) -> Result<Self, ApplicationError> {
        if let Some(root) = fallback_root.filter(|_| local_fallback_enabled()) {
            return Self::load_or_create_local(root);
        }
        match Self::load_or_create_keychain() {
            Ok(storage) => Ok(storage),
            Err(error) => {
                if let Some(root) = fallback_root.filter(|_| local_fallback_enabled()) {
                    Self::load_or_create_local(root)
                } else {
                    Err(error)
                }
            }
        }
    }

    fn load_or_create_keychain() -> Result<Self, ApplicationError> {
        if let Some(existing) = read_keychain_secret(MASTER_KEY_ACCOUNT)? {
            let master_key = decode_hex(&existing).ok_or_else(|| {
                ApplicationError::Persistence(
                    "invalid master key encoding in platform secure storage".to_string(),
                )
            })?;
            return Ok(Self {
                master_key,
                backend: SecureStorageBackend::Keychain,
            });
        }

        let rng = SystemRandom::new();
        let mut master_key = [0_u8; 32];
        rng.fill(&mut master_key).map_err(|e| {
            ApplicationError::Persistence(format!("failed to generate master key: {e}"))
        })?;
        write_keychain_secret(MASTER_KEY_ACCOUNT, &encode_hex(&master_key))?;
        Ok(Self {
            master_key,
            backend: SecureStorageBackend::Keychain,
        })
    }

    fn load_or_create_local(root: &Path) -> Result<Self, ApplicationError> {
        let storage_dir = root.join(".sbobino-secure-storage");
        std::fs::create_dir_all(storage_dir.join("secrets")).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to create local secure storage directory {}: {e}",
                storage_dir.display()
            ))
        })?;

        let master_key_path = storage_dir.join("master_key_v1");
        let master_key = if master_key_path.exists() {
            let encoded = std::fs::read_to_string(&master_key_path).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to read local master key {}: {e}",
                    master_key_path.display()
                ))
            })?;
            decode_hex(encoded.trim()).ok_or_else(|| {
                ApplicationError::Persistence("invalid local master key encoding".to_string())
            })?
        } else {
            let rng = SystemRandom::new();
            let mut generated = [0_u8; 32];
            rng.fill(&mut generated).map_err(|e| {
                ApplicationError::Persistence(format!("failed to generate master key: {e}"))
            })?;
            std::fs::write(&master_key_path, encode_hex(&generated)).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to write local master key {}: {e}",
                    master_key_path.display()
                ))
            })?;
            generated
        };

        Ok(Self {
            master_key,
            backend: SecureStorageBackend::LocalDir(storage_dir),
        })
    }

    pub fn derive_key(&self, label: &str) -> Result<[u8; 32], ApplicationError> {
        let salt = Salt::new(HKDF_SHA256, KEYCHAIN_SERVICE_NAME.as_bytes());
        let prk = salt.extract(&self.master_key);
        let info = [label.as_bytes()];
        let okm = prk
            .expand(&info, HkdfLen(32))
            .map_err(|_| ApplicationError::Persistence("failed to expand HKDF key".to_string()))?;
        let mut key = [0_u8; 32];
        okm.fill(&mut key)
            .map_err(|_| ApplicationError::Persistence("failed to fill derived key".to_string()))?;
        Ok(key)
    }

    pub fn encrypt_bytes(
        &self,
        label: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, ApplicationError> {
        let key_bytes = self.derive_key(label)?;
        let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes).map_err(|_| {
            ApplicationError::Persistence("failed to initialize AES-256-GCM key".to_string())
        })?;
        let key = LessSafeKey::new(unbound);
        let rng = SystemRandom::new();
        let mut nonce_bytes = [0_u8; 12];
        rng.fill(&mut nonce_bytes).map_err(|e| {
            ApplicationError::Persistence(format!("failed to generate encryption nonce: {e}"))
        })?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut in_out = plaintext.to_vec();
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| ApplicationError::Persistence("failed to encrypt payload".to_string()))?;

        let mut output = nonce_bytes.to_vec();
        output.extend_from_slice(&in_out);
        Ok(output)
    }

    pub fn decrypt_bytes(
        &self,
        label: &str,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, ApplicationError> {
        if ciphertext.len() < 12 {
            return Err(ApplicationError::Persistence(
                "ciphertext is too short to contain a nonce".to_string(),
            ));
        }

        let key_bytes = self.derive_key(label)?;
        let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes).map_err(|_| {
            ApplicationError::Persistence("failed to initialize AES-256-GCM key".to_string())
        })?;
        let key = LessSafeKey::new(unbound);

        let mut nonce_bytes = [0_u8; 12];
        nonce_bytes.copy_from_slice(&ciphertext[..12]);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = ciphertext[12..].to_vec();
        let plaintext = key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| ApplicationError::Persistence("failed to decrypt payload".to_string()))?;
        Ok(plaintext.to_vec())
    }

    pub fn write_secret(&self, account: &str, secret: &str) -> Result<(), ApplicationError> {
        match &self.backend {
            SecureStorageBackend::Keychain => write_keychain_secret(account, secret),
            SecureStorageBackend::LocalDir(root) => write_local_secret(root, account, secret),
        }
    }

    pub fn read_secret(&self, account: &str) -> Result<Option<String>, ApplicationError> {
        match &self.backend {
            SecureStorageBackend::Keychain => read_keychain_secret(account),
            SecureStorageBackend::LocalDir(root) => read_local_secret(root, account),
        }
    }

    pub fn delete_secret(&self, account: &str) -> Result<(), ApplicationError> {
        match &self.backend {
            SecureStorageBackend::Keychain => delete_keychain_secret(account),
            SecureStorageBackend::LocalDir(root) => delete_local_secret(root, account),
        }
    }
}

#[derive(Clone, Copy)]
struct HkdfLen(usize);

impl ring::hkdf::KeyType for HkdfLen {
    fn len(&self) -> usize {
        self.0
    }
}

#[cfg(target_os = "macos")]
fn read_keychain_secret(account: &str) -> Result<Option<String>, ApplicationError> {
    let output = Command::new("/usr/bin/security")
        .arg("find-generic-password")
        .arg("-s")
        .arg(KEYCHAIN_SERVICE_NAME)
        .arg("-a")
        .arg(account)
        .arg("-w")
        .output()
        .map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to read secret `{account}` from macOS Keychain: {e}"
            ))
        })?;

    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("could not be found") {
        return Ok(None);
    }

    Err(ApplicationError::Persistence(format!(
        "macOS Keychain lookup failed for `{account}`: {}",
        stderr.trim()
    )))
}

#[cfg(target_os = "windows")]
fn read_keychain_secret(account: &str) -> Result<Option<String>, ApplicationError> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE_NAME, account).map_err(|error| {
        ApplicationError::Persistence(format!(
            "failed to open Windows Credential Manager entry `{account}`: {error}"
        ))
    })?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(ApplicationError::Persistence(format!(
            "Windows Credential Manager lookup failed for `{account}`: {error}"
        ))),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_keychain_secret(account: &str) -> Result<Option<String>, ApplicationError> {
    Err(ApplicationError::Persistence(format!(
        "platform secure storage is unavailable for `{account}`"
    )))
}

#[cfg(target_os = "macos")]
fn write_keychain_secret(account: &str, secret: &str) -> Result<(), ApplicationError> {
    let _ = delete_keychain_secret(account);
    let output = Command::new("/usr/bin/security")
        .arg("add-generic-password")
        .arg("-U")
        .arg("-s")
        .arg(KEYCHAIN_SERVICE_NAME)
        .arg("-a")
        .arg(account)
        .arg("-w")
        .arg(secret)
        .output()
        .map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to write secret `{account}` to macOS Keychain: {e}"
            ))
        })?;

    if output.status.success() {
        return Ok(());
    }

    Err(ApplicationError::Persistence(format!(
        "macOS Keychain write failed for `{account}`: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

#[cfg(target_os = "windows")]
fn write_keychain_secret(account: &str, secret: &str) -> Result<(), ApplicationError> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE_NAME, account).map_err(|error| {
        ApplicationError::Persistence(format!(
            "failed to open Windows Credential Manager entry `{account}`: {error}"
        ))
    })?;
    entry.set_password(secret).map_err(|error| {
        ApplicationError::Persistence(format!(
            "Windows Credential Manager write failed for `{account}`: {error}"
        ))
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn write_keychain_secret(account: &str, _secret: &str) -> Result<(), ApplicationError> {
    Err(ApplicationError::Persistence(format!(
        "platform secure storage is unavailable for `{account}`"
    )))
}

#[cfg(target_os = "macos")]
fn delete_keychain_secret(account: &str) -> Result<(), ApplicationError> {
    let output = Command::new("/usr/bin/security")
        .arg("delete-generic-password")
        .arg("-s")
        .arg(KEYCHAIN_SERVICE_NAME)
        .arg("-a")
        .arg(account)
        .output()
        .map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to delete secret `{account}` from macOS Keychain: {e}"
            ))
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("could not be found") {
        return Ok(());
    }

    Err(ApplicationError::Persistence(format!(
        "macOS Keychain delete failed for `{account}`: {}",
        stderr.trim()
    )))
}

#[cfg(target_os = "windows")]
fn delete_keychain_secret(account: &str) -> Result<(), ApplicationError> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE_NAME, account).map_err(|error| {
        ApplicationError::Persistence(format!(
            "failed to open Windows Credential Manager entry `{account}`: {error}"
        ))
    })?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(ApplicationError::Persistence(format!(
            "Windows Credential Manager delete failed for `{account}`: {error}"
        ))),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn delete_keychain_secret(account: &str) -> Result<(), ApplicationError> {
    Err(ApplicationError::Persistence(format!(
        "platform secure storage is unavailable for `{account}`"
    )))
}

fn local_fallback_enabled() -> bool {
    std::env::var_os(LOCAL_FALLBACK_ENV).is_some()
}

fn local_secret_path(root: &Path, account: &str) -> PathBuf {
    root.join("secrets")
        .join(format!("{}.secret", encode_hex(account.as_bytes())))
}

fn read_local_secret(root: &Path, account: &str) -> Result<Option<String>, ApplicationError> {
    let path = local_secret_path(root, account);
    if !path.exists() {
        return Ok(None);
    }

    let secret = std::fs::read_to_string(&path).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to read local secret {}: {e}",
            path.display()
        ))
    })?;
    Ok(Some(secret))
}

fn write_local_secret(root: &Path, account: &str, secret: &str) -> Result<(), ApplicationError> {
    let path = local_secret_path(root, account);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to create local secret directory {}: {e}",
                parent.display()
            ))
        })?;
    }
    std::fs::write(&path, secret).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to write local secret {}: {e}",
            path.display()
        ))
    })
}

fn delete_local_secret(root: &Path, account: &str) -> Result<(), ApplicationError> {
    let path = local_secret_path(root, account);
    if !path.exists() {
        return Ok(());
    }

    std::fs::remove_file(&path).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to delete local secret {}: {e}",
            path.display()
        ))
    })
}

pub fn encrypt_to_file(
    storage: &SecureStorage,
    label: &str,
    path: &Path,
    plaintext: &[u8],
) -> Result<(), ApplicationError> {
    let ciphertext = storage.encrypt_bytes(label, plaintext)?;
    std::fs::write(path, ciphertext).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to write encrypted file {}: {e}",
            path.display()
        ))
    })
}

pub fn decrypt_from_file(
    storage: &SecureStorage,
    label: &str,
    path: &Path,
) -> Result<Vec<u8>, ApplicationError> {
    let ciphertext = std::fs::read(path).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to read encrypted file {}: {e}",
            path.display()
        ))
    })?;
    storage.decrypt_bytes(label, &ciphertext)
}

pub fn encrypt_file_with_password(
    input_path: &Path,
    output_path: &Path,
    password: &str,
) -> Result<(), ApplicationError> {
    if password.trim().is_empty() {
        return Err(ApplicationError::Validation(
            "backup password cannot be empty".to_string(),
        ));
    }

    let input = std::fs::File::open(input_path).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to open backup input {}: {e}",
            input_path.display()
        ))
    })?;
    let mut reader = BufReader::new(input);

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to create backup destination directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let output = std::fs::File::create(output_path).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to create encrypted backup {}: {e}",
            output_path.display()
        ))
    })?;
    let mut writer = BufWriter::new(output);

    let rng = SystemRandom::new();
    let mut salt = [0_u8; BACKUP_SALT_LEN];
    let mut nonce_prefix = [0_u8; BACKUP_NONCE_PREFIX_LEN];
    rng.fill(&mut salt).map_err(|e| {
        ApplicationError::Persistence(format!("failed to generate backup salt: {e}"))
    })?;
    rng.fill(&mut nonce_prefix).map_err(|e| {
        ApplicationError::Persistence(format!("failed to generate backup nonce prefix: {e}"))
    })?;

    writer.write_all(BACKUP_FILE_MAGIC).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to write backup header to {}: {e}",
            output_path.display()
        ))
    })?;
    writer
        .write_all(&BACKUP_PBKDF2_ITERATIONS.to_be_bytes())
        .map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to write backup iteration header to {}: {e}",
                output_path.display()
            ))
        })?;
    writer
        .write_all(&(BACKUP_CHUNK_SIZE as u32).to_be_bytes())
        .map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to write backup chunk header to {}: {e}",
                output_path.display()
            ))
        })?;
    writer.write_all(&salt).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to write backup salt to {}: {e}",
            output_path.display()
        ))
    })?;
    writer.write_all(&nonce_prefix).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to write backup nonce prefix to {}: {e}",
            output_path.display()
        ))
    })?;

    let key = password_key(password, &salt, BACKUP_PBKDF2_ITERATIONS)?;
    let cipher = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key).map_err(|_| {
        ApplicationError::Persistence("failed to initialize backup cipher".to_string())
    })?);
    let mut buffer = vec![0_u8; BACKUP_CHUNK_SIZE];
    let mut chunk_index = 0_u32;

    loop {
        let read = reader.read(&mut buffer).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to read backup source {}: {e}",
                input_path.display()
            ))
        })?;
        if read == 0 {
            break;
        }

        let mut in_out = buffer[..read].to_vec();
        cipher
            .seal_in_place_append_tag(
                backup_nonce(nonce_prefix, chunk_index),
                Aad::from(chunk_index.to_be_bytes()),
                &mut in_out,
            )
            .map_err(|_| {
                ApplicationError::Persistence("failed to encrypt backup chunk".to_string())
            })?;

        writer
            .write_all(&(in_out.len() as u32).to_be_bytes())
            .and_then(|_| writer.write_all(&in_out))
            .map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to write backup chunk to {}: {e}",
                    output_path.display()
                ))
            })?;
        chunk_index = chunk_index.saturating_add(1);
    }

    writer.flush().map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to flush encrypted backup {}: {e}",
            output_path.display()
        ))
    })
}

pub fn decrypt_file_with_password(
    input_path: &Path,
    output_path: &Path,
    password: &str,
) -> Result<(), ApplicationError> {
    if password.trim().is_empty() {
        return Err(ApplicationError::Validation(
            "backup password cannot be empty".to_string(),
        ));
    }

    let input = std::fs::File::open(input_path).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to open encrypted backup {}: {e}",
            input_path.display()
        ))
    })?;
    let mut reader = BufReader::new(input);

    let mut magic = [0_u8; 8];
    reader.read_exact(&mut magic).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to read backup header from {}: {e}",
            input_path.display()
        ))
    })?;
    if &magic != BACKUP_FILE_MAGIC {
        return Err(ApplicationError::Validation(
            "unsupported backup file format".to_string(),
        ));
    }

    let iterations = read_u32_be(&mut reader, input_path, "PBKDF2 iteration count")?;
    let chunk_size = read_u32_be(&mut reader, input_path, "chunk size")? as usize;
    if chunk_size == 0 {
        return Err(ApplicationError::Validation(
            "backup file contains an invalid chunk size".to_string(),
        ));
    }

    let mut salt = [0_u8; BACKUP_SALT_LEN];
    let mut nonce_prefix = [0_u8; BACKUP_NONCE_PREFIX_LEN];
    reader.read_exact(&mut salt).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to read backup salt from {}: {e}",
            input_path.display()
        ))
    })?;
    reader.read_exact(&mut nonce_prefix).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to read backup nonce prefix from {}: {e}",
            input_path.display()
        ))
    })?;

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to create decrypted backup directory {}: {e}",
                parent.display()
            ))
        })?;
    }
    let output = std::fs::File::create(output_path).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to create decrypted backup {}: {e}",
            output_path.display()
        ))
    })?;
    let mut writer = BufWriter::new(output);

    let key = password_key(password, &salt, iterations)?;
    let cipher = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key).map_err(|_| {
        ApplicationError::Persistence("failed to initialize backup cipher".to_string())
    })?);

    let mut chunk_index = 0_u32;
    loop {
        let mut len_bytes = [0_u8; 4];
        match reader.read_exact(&mut len_bytes) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => {
                return Err(ApplicationError::Persistence(format!(
                    "failed to read backup chunk header from {}: {error}",
                    input_path.display()
                )))
            }
        }

        let chunk_len = u32::from_be_bytes(len_bytes) as usize;
        if chunk_len == 0 || chunk_len > chunk_size + 32 {
            return Err(ApplicationError::Validation(
                "backup file contains an invalid encrypted chunk length".to_string(),
            ));
        }

        let mut in_out = vec![0_u8; chunk_len];
        reader.read_exact(&mut in_out).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to read backup chunk from {}: {e}",
                input_path.display()
            ))
        })?;

        let plaintext = cipher
            .open_in_place(
                backup_nonce(nonce_prefix, chunk_index),
                Aad::from(chunk_index.to_be_bytes()),
                &mut in_out,
            )
            .map_err(|_| {
                ApplicationError::Validation(
                    "backup password is invalid or the backup file is corrupted".to_string(),
                )
            })?;
        writer.write_all(plaintext).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to write decrypted backup {}: {e}",
                output_path.display()
            ))
        })?;
        chunk_index = chunk_index.saturating_add(1);
    }

    writer.flush().map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to flush decrypted backup {}: {e}",
            output_path.display()
        ))
    })
}

pub fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

fn decode_hex(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }

    let mut bytes = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks(2).enumerate() {
        let piece = std::str::from_utf8(chunk).ok()?;
        bytes[index] = u8::from_str_radix(piece, 16).ok()?;
    }
    Some(bytes)
}

fn password_key(
    password: &str,
    salt: &[u8; BACKUP_SALT_LEN],
    iterations: u32,
) -> Result<[u8; 32], ApplicationError> {
    let iterations = NonZeroU32::new(iterations).ok_or_else(|| {
        ApplicationError::Validation("backup iteration count cannot be zero".to_string())
    })?;
    let mut key = [0_u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        password.as_bytes(),
        &mut key,
    );
    Ok(key)
}

fn backup_nonce(prefix: [u8; BACKUP_NONCE_PREFIX_LEN], chunk_index: u32) -> Nonce {
    let mut bytes = [0_u8; 12];
    bytes[..BACKUP_NONCE_PREFIX_LEN].copy_from_slice(&prefix);
    bytes[BACKUP_NONCE_PREFIX_LEN..].copy_from_slice(&chunk_index.to_be_bytes());
    Nonce::assume_unique_for_key(bytes)
}

fn read_u32_be(
    reader: &mut BufReader<std::fs::File>,
    input_path: &Path,
    label: &str,
) -> Result<u32, ApplicationError> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to read backup {label} from {}: {e}",
            input_path.display()
        ))
    })?;
    Ok(u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::{decrypt_file_with_password, encrypt_file_with_password};
    use tempfile::tempdir;

    #[test]
    fn password_backup_file_roundtrip_restores_original_bytes() {
        let temp = tempdir().expect("tempdir");
        let input = temp.path().join("plain.bin");
        let encrypted = temp.path().join("backup.sbobino-backup");
        let output = temp.path().join("restored.bin");

        std::fs::write(&input, b"portable backup payload").expect("write input");
        encrypt_file_with_password(&input, &encrypted, "correct horse battery staple")
            .expect("encrypt backup");
        decrypt_file_with_password(&encrypted, &output, "correct horse battery staple")
            .expect("decrypt backup");

        let restored = std::fs::read(&output).expect("read restored");
        assert_eq!(restored, b"portable backup payload");
    }

    #[test]
    fn password_backup_file_rejects_wrong_password() {
        let temp = tempdir().expect("tempdir");
        let input = temp.path().join("plain.bin");
        let encrypted = temp.path().join("backup.sbobino-backup");
        let output = temp.path().join("restored.bin");

        std::fs::write(&input, b"portable backup payload").expect("write input");
        encrypt_file_with_password(&input, &encrypted, "correct horse battery staple")
            .expect("encrypt backup");
        let error = decrypt_file_with_password(&encrypted, &output, "wrong password")
            .expect_err("wrong password should fail");

        assert!(error
            .to_string()
            .contains("backup password is invalid or the backup file is corrupted"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_credential_manager_roundtrip() {
        let account = format!(
            "test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should follow epoch")
                .as_nanos()
        );
        super::write_keychain_secret(&account, "sbobino-test-secret")
            .expect("credential should write");
        assert_eq!(
            super::read_keychain_secret(&account).expect("credential should read"),
            Some("sbobino-test-secret".to_string())
        );
        super::delete_keychain_secret(&account).expect("credential should delete");
        assert_eq!(
            super::read_keychain_secret(&account).expect("deleted credential should read"),
            None
        );
    }
}
