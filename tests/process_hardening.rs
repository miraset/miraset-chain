//! Process hardening (key custody proposal D4, threat K4). Own test binary: it changes
//! process-wide limits.

#[cfg(target_os = "linux")]
#[test]
fn test_disable_core_dumps_sets_zero_core_limit_and_non_dumpable() {
    miraset_chain::process_hardening::disable_core_dumps().expect("hardening applies");

    let limits = std::fs::read_to_string("/proc/self/limits").unwrap();
    let core = limits
        .lines()
        .find(|line| line.starts_with("Max core file size"))
        .expect("core limit line");
    let values: Vec<&str> = core.split_whitespace().skip(4).take(2).collect();
    assert_eq!(values, ["0", "0"], "{core}");

    // SAFETY: `PR_GET_DUMPABLE` takes no pointers and only returns the flag.
    let dumpable = unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) };
    assert_eq!(dumpable, 0);

    // Idempotent: a second call (e.g. a subcommand after `main`) still succeeds.
    miraset_chain::process_hardening::disable_core_dumps().expect("idempotent");
}
