//! Tests for the measurement-campaign metrics (TODO 8.5.3).

use super::*;

fn value<'a>(text: &'a str, series: &str) -> &'a str {
    text.lines()
        .find_map(|line| {
            line.strip_prefix(series)
                .and_then(|rest| rest.strip_prefix(' '))
        })
        .unwrap_or_else(|| panic!("{series} missing in:\n{text}"))
}

#[test]
fn test_histogram_buckets_are_cumulative_and_scaled() {
    let histogram = Histogram::new(&[100, 1_000]);
    for v in [50, 100, 500, 5_000] {
        histogram.observe(v);
    }
    let mut out = String::new();
    histogram.render(&mut out, "x_seconds", "help", 1000.0);
    assert_eq!(value(&out, "x_seconds_bucket{le=\"0.1\"}"), "2");
    assert_eq!(value(&out, "x_seconds_bucket{le=\"1\"}"), "3");
    assert_eq!(value(&out, "x_seconds_bucket{le=\"+Inf\"}"), "4");
    assert_eq!(value(&out, "x_seconds_sum"), "5.65");
    assert_eq!(value(&out, "x_seconds_count"), "4");
}

#[test]
fn test_block_shape_view_health_and_traffic_are_exported() {
    let metrics = RuntimeMetrics::default();
    metrics.record_block(0, 0, 120);
    metrics.record_block(40, 2, 9_000);
    let start = Instant::now();
    metrics.record_commit_time(start, true);
    metrics.record_commit_time(start + std::time::Duration::from_millis(300), true);
    metrics.record_unreadable_block();
    metrics.record_view(17);
    metrics.record_view(12);
    metrics.record_view_timeout();
    metrics.record_timeout_certificate();
    let traffic = Arc::new(TransportTraffic::default());
    traffic.record_sent(10);
    traffic.record_received(25);
    metrics.attach_traffic(traffic);

    let mut out = String::new();
    metrics.render(&mut out);
    assert_eq!(value(&out, "miraset_empty_blocks_total"), "1");
    assert_eq!(value(&out, "miraset_block_txs_count"), "2");
    assert_eq!(value(&out, "miraset_block_txs_sum"), "40");
    assert_eq!(value(&out, "miraset_block_payload_bytes_sum"), "9120");
    assert_eq!(value(&out, "miraset_commit_interval_seconds_count"), "1");
    assert_eq!(
        value(&out, "miraset_commit_interval_seconds_bucket{le=\"0.25\"}"),
        "0"
    );
    assert_eq!(
        value(&out, "miraset_commit_interval_seconds_bucket{le=\"0.5\"}"),
        "1"
    );
    assert_eq!(value(&out, "miraset_unreadable_blocks_total"), "1");
    assert_eq!(value(&out, "miraset_view"), "17");
    assert_eq!(value(&out, "miraset_view_timeouts_total"), "1");
    assert_eq!(value(&out, "miraset_timeout_certificates_total"), "1");
    assert_eq!(
        value(&out, "miraset_p2p_bytes_total{direction=\"received\"}"),
        "25"
    );
}

#[test]
fn test_storage_sizes_are_read_at_scrape() {
    let dir = tempfile::tempdir().unwrap();
    let chain = dir.path().join("chain.redb");
    std::fs::write(&chain, vec![0u8; 4096]).unwrap();
    let metrics = RuntimeMetrics::default();
    metrics.attach_storage(StorageFiles {
        chain_db: chain.clone(),
        signing_guard: Some(dir.path().join("missing.redb")),
    });
    let mut out = String::new();
    metrics.render(&mut out);
    assert_eq!(
        value(&out, "miraset_storage_bytes{file=\"chain_db\"}"),
        "4096"
    );
    assert!(
        !out.contains("file=\"signing_guard\""),
        "missing file is skipped"
    );
    std::fs::write(&chain, vec![0u8; 8192]).unwrap();
    let mut out = String::new();
    metrics.render(&mut out);
    assert_eq!(
        value(&out, "miraset_storage_bytes{file=\"chain_db\"}"),
        "8192"
    );
}

#[test]
fn test_process_stats_parse_procfs_text() {
    let status = "Name:\tmiraset\nVmRSS:\t   2048 kB\nThreads:\t14\n";
    // pid (comm with ) inside) state ppid ... utime=250 stime=50 at fields 14 and 15.
    let stat = "42 (miraset (x)) S 1 1 1 0 -1 4194560 1 0 0 0 250 50 0 0 20 0 14 0";
    let io = "rchar: 1\nwrite_bytes: 65536\n";
    let stats = ProcessStats::parse(status, stat, Some(io), 9, 100).unwrap();
    assert_eq!(
        stats,
        ProcessStats {
            resident_bytes: 2048 * 1024,
            cpu_seconds: 3.0,
            open_fds: 9,
            threads: 14,
            write_bytes: Some(65536),
        }
    );
    assert!(ProcessStats::parse("", stat, None, 0, 100).is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn test_process_stats_are_read_from_this_process() {
    let stats = ProcessStats::read().expect("procfs");
    assert!(stats.resident_bytes > 0 && stats.threads > 0 && stats.open_fds > 0);
}

/// Block-sync commits do not form intervals: the burst resets the interval instead of filling
/// the lowest buckets, and the first consensus commit after it starts a new interval.
#[test]
fn test_sync_commits_do_not_form_commit_intervals() {
    let metrics = RuntimeMetrics::default();
    let t = Instant::now();
    let ms = std::time::Duration::from_millis;
    metrics.record_commit_time(t, true);
    for i in 1..=50 {
        metrics.record_commit_time(t + ms(i), false);
    }
    metrics.record_commit_time(t + ms(5_000), true);
    metrics.record_commit_time(t + ms(5_400), true);
    let mut out = String::new();
    metrics.render(&mut out);
    assert_eq!(value(&out, "miraset_commit_interval_seconds_count"), "1");
    assert_eq!(value(&out, "miraset_commit_interval_seconds_sum"), "0.4");
}

#[test]
fn test_observations_above_every_bound_land_in_inf_only() {
    let histogram = Histogram::new(&[10]);
    histogram.observe(5);
    histogram.observe(50);
    let mut out = String::new();
    histogram.render(&mut out, "h", "help", 1.0);
    assert_eq!(value(&out, "h_bucket{le=\"10\"}"), "1");
    assert_eq!(value(&out, "h_bucket{le=\"+Inf\"}"), "2");
    assert_eq!(value(&out, "h_count"), "2");
}

#[test]
fn test_block_timings_are_exported_in_seconds() {
    let metrics = RuntimeMetrics::default();
    let ms = std::time::Duration::from_millis;
    metrics.record_block_production(ms(900), ms(400));
    metrics.record_block_validation(ms(600));
    let mut out = String::new();
    metrics.render(&mut out);
    assert_eq!(value(&out, "miraset_block_produce_wait_seconds_sum"), "0.9");
    assert_eq!(
        value(&out, "miraset_block_produce_build_seconds_sum"),
        "0.4"
    );
    assert_eq!(value(&out, "miraset_block_validate_seconds_count"), "1");
    assert_eq!(
        value(&out, "miraset_block_validate_seconds_bucket{le=\"0.5\"}"),
        "0"
    );
    assert_eq!(
        value(&out, "miraset_block_validate_seconds_bucket{le=\"0.75\"}"),
        "1"
    );
}
