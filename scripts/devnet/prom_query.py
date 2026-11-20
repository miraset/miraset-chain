"""Query the observe-profile Prometheus for measurement runs (TODO 8.5.3). Usage: prom_query.py WINDOW (e.g. 90s)."""
import json, os, sys, urllib.parse, urllib.request

PROM = f"http://127.0.0.1:{os.environ.get('MIRASET_PROMETHEUS_PORT', '9090')}/api/v1/query"
WINDOW = sys.argv[1] if len(sys.argv) > 1 else "25m"

def q(expr):
    url = PROM + "?" + urllib.parse.urlencode({"query": expr})
    with urllib.request.urlopen(url, timeout=10) as r:
        data = json.load(r)["data"]["result"]
    return {row["metric"].get("instance", "all"): float(row["value"][1]) for row in data}

queries = {
    "blocks_per_s": f"rate(miraset_commits_total[{WINDOW}])",
    "empty_blocks_per_s": f"rate(miraset_empty_blocks_total[{WINDOW}])",
    "payload_bytes_per_block": f"rate(miraset_block_payload_bytes_sum[{WINDOW}]) / rate(miraset_block_payload_bytes_count[{WINDOW}])",
    "commit_interval_p50_s": f"histogram_quantile(0.5, rate(miraset_commit_interval_seconds_bucket[{WINDOW}]))",
    "commit_interval_p99_s": f"histogram_quantile(0.99, rate(miraset_commit_interval_seconds_bucket[{WINDOW}]))",
    "txs_per_s": f"rate(miraset_block_txs_sum[{WINDOW}])",
    "receipts_per_s": f"rate(miraset_block_receipts_sum[{WINDOW}])",
    "max_payload_bytes_le": f"histogram_quantile(0.99, rate(miraset_block_payload_bytes_bucket[{WINDOW}]))",
    "timeout_certs_per_s": f"rate(miraset_timeout_certificates_total[{WINDOW}])",
    "produce_wait_p50_s": f"histogram_quantile(0.5, rate(miraset_block_produce_wait_seconds_bucket[{WINDOW}]))",
    "produce_build_p50_s": f"histogram_quantile(0.5, rate(miraset_block_produce_build_seconds_bucket[{WINDOW}]))",
    "produce_build_p99_s": f"histogram_quantile(0.99, rate(miraset_block_produce_build_seconds_bucket[{WINDOW}]))",
    "produce_wait_mean_s": f"rate(miraset_block_produce_wait_seconds_sum[{WINDOW}]) / rate(miraset_block_produce_wait_seconds_count[{WINDOW}])",
    "produce_build_mean_s": f"rate(miraset_block_produce_build_seconds_sum[{WINDOW}]) / rate(miraset_block_produce_build_seconds_count[{WINDOW}])",
    "validate_mean_s": f"rate(miraset_block_validate_seconds_sum[{WINDOW}]) / rate(miraset_block_validate_seconds_count[{WINDOW}])",
    "validate_decode_mean_s": f"rate(miraset_block_validate_decode_seconds_sum[{WINDOW}]) / rate(miraset_block_validate_decode_seconds_count[{WINDOW}])",
    "validate_signatures_mean_s": f"rate(miraset_block_validate_signatures_seconds_sum[{WINDOW}]) / rate(miraset_block_validate_signatures_seconds_count[{WINDOW}])",
    "validate_execute_mean_s": f"rate(miraset_block_validate_execute_seconds_sum[{WINDOW}]) / rate(miraset_block_validate_execute_seconds_count[{WINDOW}])",
    "validate_finish_mean_s": f"rate(miraset_block_validate_finish_seconds_sum[{WINDOW}]) / rate(miraset_block_validate_finish_seconds_count[{WINDOW}])",
    "validate_p99_s": f"histogram_quantile(0.99, rate(miraset_block_validate_seconds_bucket[{WINDOW}]))",
    "txs_per_block_max_le": f"histogram_quantile(1.0, rate(miraset_block_txs_bucket[{WINDOW}]))",
    "view_timeouts_per_s": f"rate(miraset_view_timeouts_total[{WINDOW}])",
    "chain_db_bytes": 'miraset_storage_bytes{file="chain_db"}',
    "chain_db_growth_bytes_per_h": f'deriv(miraset_storage_bytes{{file="chain_db"}}[{WINDOW}]) * 3600',
    "guard_bytes": 'miraset_storage_bytes{file="signing_guard"}',
    "rss_bytes": "miraset_process_resident_bytes",
    "rss_growth_bytes_per_h": f"deriv(miraset_process_resident_bytes[{WINDOW}]) * 3600",
    "cpu_cores": f"rate(miraset_process_cpu_seconds_total[{WINDOW}])",
    "disk_write_bytes_per_s": f"rate(miraset_process_write_bytes_total[{WINDOW}])",
    "open_fds": "miraset_process_open_fds",
    "threads": "miraset_process_threads",
    "p2p_sent_bytes_per_s": f'rate(miraset_p2p_bytes_total{{direction="sent"}}[{WINDOW}])',
    "p2p_received_bytes_per_s": f'rate(miraset_p2p_bytes_total{{direction="received"}}[{WINDOW}])',
    "committed_height": "miraset_committed_height",
    "p2p_rejected": "sum by (instance) (miraset_p2p_rejected_total)",
    "unreadable_blocks": "miraset_unreadable_blocks_total",
}
result = {name: q(expr) for name, expr in queries.items()}
print(json.dumps(result, indent=1, sort_keys=True))
