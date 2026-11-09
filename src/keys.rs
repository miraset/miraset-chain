//! Validator and account key files (TODO 8.3.4, `docs/adr/proposal-validator-key-custody.md` D4).
//!
//! A key file holds a hex-encoded Ed25519 secret. It is created with mode 0600 and never
//! overwritten. Reads fail closed unless the path is a regular file (not a symlink) owned by the
//! process user with no group or other access. Every buffer that holds secret bytes is zeroized
//! on drop. Secrets never appear in error messages.

use ed25519_dalek::SigningKey;
use rand::RngExt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use zeroize::Zeroizing;

pub const SECRET_KEY_BYTES: usize = 32;
/// Owner-only permissions for created key files.
const KEY_FILE_MODE: u32 = 0o600;
/// Permission bits that must be clear on a key file (any group or other access).
const GROUP_OTHER_BITS: u32 = 0o077;
/// Hex encoding doubles the length; allow trailing whitespace but nothing large.
const MAX_KEY_FILE_BYTES: u64 = 256;

/// Generate a fresh random Ed25519 signing key.
pub fn generate_signing_key() -> SigningKey {
    let mut secret = Zeroizing::new([0u8; SECRET_KEY_BYTES]);
    rand::rng().fill(&mut *secret);
    SigningKey::from_bytes(&secret)
}

/// Write `key` as hex to a new file with mode 0600. Refuses to overwrite an existing file or
/// to follow a symlink at `path`.
pub fn write_key_file(path: &Path, key: &SigningKey) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(KEY_FILE_MODE);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("cannot create key file {}: {e}", path.display()))?;
    let secret = Zeroizing::new(key.to_bytes());
    let encoded = Zeroizing::new(hex::encode(*secret));
    file.write_all(encoded.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("cannot write key file {}: {e}", path.display()))
}

/// Read a hex-encoded Ed25519 secret key file, failing closed on anything unexpected.
pub fn read_key_file(path: &Path) -> Result<SigningKey, String> {
    let file = open_key_file(path)?;
    let mut content = Zeroizing::new(String::new());
    file.take(MAX_KEY_FILE_BYTES)
        .read_to_string(&mut content)
        .map_err(|e| format!("cannot read key file {}: {e}", path.display()))?;
    let decoded = Zeroizing::new(hex::decode(content.trim()).map_err(|_| malformed(path))?);
    let mut secret = Zeroizing::new([0u8; SECRET_KEY_BYTES]);
    if decoded.len() != SECRET_KEY_BYTES {
        return Err(malformed(path));
    }
    secret.copy_from_slice(&decoded);
    Ok(SigningKey::from_bytes(&secret))
}

fn malformed(path: &Path) -> String {
    format!(
        "key file {} must contain {SECRET_KEY_BYTES} hex-encoded bytes",
        path.display()
    )
}

/// Facts about an opened key file that decide whether it may be used.
#[derive(Clone, Copy, Debug)]
pub struct KeyFileFacts {
    pub is_regular_file: bool,
    pub mode: u32,
    pub owner_uid: u32,
    pub process_uid: u32,
}

/// The key file policy, separate from I/O so every branch is testable.
pub fn check_key_file(path: &Path, facts: KeyFileFacts) -> Result<(), String> {
    if !facts.is_regular_file {
        return Err(format!("key file {} is not a regular file", path.display()));
    }
    if facts.owner_uid != facts.process_uid {
        return Err(format!(
            "key file {} is owned by uid {}, not by the process user (uid {})",
            path.display(),
            facts.owner_uid,
            facts.process_uid
        ));
    }
    if facts.mode & GROUP_OTHER_BITS != 0 {
        return Err(format!(
            "key file {} is accessible by group/others (mode {:o}); run chmod 600",
            path.display(),
            facts.mode & 0o777
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn open_key_file(path: &Path) -> Result<File, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP) => format!("key file {} must not be a symlink", path.display()),
            _ => format!("cannot open key file {}: {e}", path.display()),
        })?;
    // Check the opened handle, not the path, so the file cannot be swapped in between.
    let metadata = file
        .metadata()
        .map_err(|e| format!("cannot stat key file {}: {e}", path.display()))?;
    check_key_file(
        path,
        KeyFileFacts {
            is_regular_file: metadata.file_type().is_file(),
            mode: metadata.mode(),
            owner_uid: metadata.uid(),
            process_uid: effective_uid(),
        },
    )?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_key_file(path: &Path) -> Result<File, String> {
    File::open(path).map_err(|e| format!("cannot open key file {}: {e}", path.display()))
}

#[cfg(unix)]
pub fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions, cannot fail and touches no memory we own.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
