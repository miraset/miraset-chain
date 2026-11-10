//! Latency summaries for load reports (TODO 8.5.3).

use serde::Serialize;
use std::time::Duration;

/// Nearest-rank percentiles of a set of latencies, in milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LatencySummary {
    pub count: usize,
    pub p50_ms: Option<f64>,
    pub p90_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub mean_ms: Option<f64>,
}

impl LatencySummary {
    pub fn from_samples(samples: &[Duration]) -> Self {
        let mut sorted: Vec<Duration> = samples.to_vec();
        sorted.sort_unstable();
        let mean_ms = (!sorted.is_empty()).then(|| {
            let total: f64 = sorted.iter().map(|d| as_ms(*d)).sum();
            round_ms(total / sorted.len() as f64)
        });
        Self {
            count: sorted.len(),
            p50_ms: percentile(&sorted, 50.0),
            p90_ms: percentile(&sorted, 90.0),
            p95_ms: percentile(&sorted, 95.0),
            p99_ms: percentile(&sorted, 99.0),
            max_ms: sorted.last().map(|d| round_ms(as_ms(*d))),
            mean_ms,
        }
    }
}

/// Nearest-rank percentile (`p` in 0..=100) of an ascending slice; `None` when empty.
pub fn percentile(sorted: &[Duration], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.clamp(1, sorted.len()) - 1;
    Some(round_ms(as_ms(sorted[index])))
}

fn as_ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn round_ms(ms: f64) -> f64 {
    (ms * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(values: &[u64]) -> Vec<Duration> {
        values.iter().map(|v| Duration::from_millis(*v)).collect()
    }

    #[test]
    fn test_percentile_uses_nearest_rank() {
        let sorted = ms(&[10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
        assert_eq!(percentile(&sorted, 50.0), Some(50.0));
        assert_eq!(percentile(&sorted, 90.0), Some(90.0));
        assert_eq!(percentile(&sorted, 99.0), Some(100.0));
        assert_eq!(percentile(&sorted, 0.0), Some(10.0));
    }

    #[test]
    fn test_summary_of_no_samples_has_no_values() {
        let summary = LatencySummary::from_samples(&[]);
        assert_eq!(summary.count, 0);
        assert_eq!(summary.p50_ms, None);
        assert_eq!(summary.max_ms, None);
        assert_eq!(summary.mean_ms, None);
    }

    #[test]
    fn test_summary_sorts_unordered_samples() {
        let summary = LatencySummary::from_samples(&ms(&[300, 100, 200]));
        assert_eq!(summary.count, 3);
        assert_eq!(summary.p50_ms, Some(200.0));
        assert_eq!(summary.max_ms, Some(300.0));
        assert_eq!(summary.mean_ms, Some(200.0));
    }
}
