//! Unit tests for key file handling (key custody proposal D4).

use super::*;
use std::path::PathBuf;

const OWNER: u32 = 10001;

fn facts(mode: u32) -> KeyFileFacts {
    KeyFileFacts {
        is_regular_file: true,
        mode,
        owner_uid: OWNER,
        process_uid: OWNER,
    }
}

fn path() -> PathBuf {
    PathBuf::from("/keys/node0.key")
}

#[test]
fn test_policy_accepts_owner_only_regular_file() {
    assert!(check_key_file(&path(), facts(0o100600)).is_ok());
    assert!(check_key_file(&path(), facts(0o100400)).is_ok());
}

#[test]
fn test_policy_rejects_group_or_other_access() {
    for mode in [0o100640, 0o100604, 0o100660, 0o100644, 0o100610] {
        let err = check_key_file(&path(), facts(mode)).unwrap_err();
        assert!(err.contains("group/others"), "{mode:o}: {err}");
    }
}

#[test]
fn test_policy_rejects_other_owner_and_non_regular_files() {
    let foreign = KeyFileFacts {
        owner_uid: 0,
        ..facts(0o100600)
    };
    assert!(
        check_key_file(&path(), foreign)
            .unwrap_err()
            .contains("owned by uid 0")
    );

    let directory = KeyFileFacts {
        is_regular_file: false,
        ..facts(0o040700)
    };
    assert!(
        check_key_file(&path(), directory)
            .unwrap_err()
            .contains("not a regular file")
    );
}

#[cfg(unix)]
mod files {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn test_round_trip_creates_owner_only_file_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.key");
        let key = generate_signing_key();
        write_key_file(&path, &key).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(read_key_file(&path).unwrap().to_bytes(), key.to_bytes());
        assert!(write_key_file(&path, &key).is_err(), "must not overwrite");
    }

    #[test]
    fn test_read_refuses_symlink_even_to_a_valid_key() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.key");
        write_key_file(&target, &generate_signing_key()).unwrap();
        let link = dir.path().join("link.key");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = read_key_file(&link).unwrap_err();
        assert!(err.contains("symlink"), "{err}");
    }

    #[test]
    fn test_write_refuses_to_follow_existing_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        let link = dir.path().join("k.key");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        assert!(write_key_file(&link, &generate_signing_key()).is_err());
        assert!(!elsewhere.exists(), "nothing written through the symlink");
    }

    #[test]
    fn test_read_refuses_bad_mode_bad_length_and_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("k.key");
        write_key_file(&key_path, &generate_signing_key()).unwrap();
        set_mode(&key_path, 0o644);
        assert!(
            read_key_file(&key_path)
                .unwrap_err()
                .contains("group/others")
        );

        let short = dir.path().join("short.key");
        std::fs::write(&short, "abcd").unwrap();
        set_mode(&short, 0o600);
        assert!(read_key_file(&short).unwrap_err().contains("hex-encoded"));

        let huge = dir.path().join("huge.key");
        std::fs::write(&huge, "ab".repeat(1024)).unwrap();
        set_mode(&huge, 0o600);
        assert!(read_key_file(&huge).is_err());
    }

    #[test]
    fn test_errors_never_contain_the_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.key");
        let key = generate_signing_key();
        write_key_file(&path, &key).unwrap();
        let secret_hex = hex::encode(key.to_bytes());

        set_mode(&path, 0o644);
        let mode_err = read_key_file(&path).unwrap_err();
        // A key with a typo: still secret-looking material that must not be echoed back.
        let typo = dir.path().join("typo.key");
        std::fs::write(&typo, format!("{secret_hex}zz")).unwrap();
        set_mode(&typo, 0o600);
        let parse_err = read_key_file(&typo).unwrap_err();

        for err in [mode_err, parse_err] {
            assert!(!err.contains(&secret_hex), "{err}");
            assert!(!err.contains(&secret_hex[..16]), "{err}");
        }
    }
}
