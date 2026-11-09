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
