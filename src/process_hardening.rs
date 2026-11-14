//! Process hardening for anything that loads a secret key (key custody proposal D4, threat K4).
//!
//! - `RLIMIT_CORE = 0`: a crash does not write the key to a core file.
//! - Linux `PR_SET_DUMPABLE = 0`: no core dumps, and other processes running as the same user
//!   cannot ptrace this process or read its memory through `/proc/<pid>/mem`.
//!
//! On non-Unix targets this is a documented no-op.

/// Apply the hardening once at process start, before any key is read. Fails closed.
pub fn disable_core_dumps() -> Result<(), String> {
    #[cfg(unix)]
    {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `setrlimit` reads one fully initialized `rlimit` by pointer for the duration
        // of the call; the pointer is valid and nothing is retained.
        if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } != 0 {
            return Err(format!(
                "cannot disable core dumps: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `PR_SET_DUMPABLE` takes plain integer arguments and touches no memory.
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
            return Err(format!(
                "cannot mark process non-dumpable: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

/// Raise the soft open-file limit to the hard limit and return the soft limit now in force
/// (`None` where it cannot be read). Container runtimes often start processes with a soft
/// limit of 1 024, which a validator's RPC listener exhausted under load (stress test 2,
/// `docs/performance-baseline.md` finding 26).
pub fn raise_open_file_limit() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `getrlimit` writes one `rlimit` through a valid, exclusive pointer.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
            return None;
        }
        if limit.rlim_cur < limit.rlim_max {
            let raised = libc::rlimit {
                rlim_cur: limit.rlim_max,
                rlim_max: limit.rlim_max,
            };
            // SAFETY: as for `setrlimit` above; the pointer is valid for the call.
            if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } == 0 {
                limit = raised;
            }
        }
        #[allow(clippy::unnecessary_cast)] // `rlim_t` is not `u64` on every Unix
        Some(limit.rlim_cur as u64)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// The soft open-file limit in force (`None` where it cannot be read).
pub fn open_file_limit() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `getrlimit` writes one `rlimit` through a valid, exclusive pointer.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)] // `rlim_t` is not `u64` on every Unix
        Some(limit.rlim_cur as u64)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(test)]
mod fd_limit_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn test_open_file_limit_is_raised_to_the_hard_limit() {
        let soft = raise_open_file_limit().expect("readable on unix");
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: valid exclusive pointer for the call.
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
            0
        );
        assert_eq!(open_file_limit(), Some(soft));
        assert_eq!(
            limit.rlim_cur, limit.rlim_max,
            "soft limit raised to the hard limit"
        );
    }
}
