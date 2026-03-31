/// In-memory metrics collection for Shadow IDE.
/// Tracks tokens/s, tool latencies, error counts, and request statistics.
use std::collections::{HashMap, VecDeque};

pub struct MetricsData {
    pub tokens_per_second: VecDeque<f64>,
    pub tool_latencies: HashMap<String, VecDeque<u64>>,
    pub error_counts: HashMap<String, u64>,
    pub request_count: u64,
    pub total_tokens: u64,
}

impl MetricsData {
    pub fn new() -> Self {
        MetricsData {
            tokens_per_second: VecDeque::new(),
            tool_latencies: HashMap::new(),
            error_counts: HashMap::new(),
            request_count: 0,
            total_tokens: 0,
        }
    }
}

pub struct MetricsStore;

static METRICS_STORE: std::sync::LazyLock<std::sync::Mutex<MetricsData>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(MetricsData::new()));

impl MetricsStore {
    pub fn record_tokens_per_second(tps: f64) {
        if let Ok(mut store) = METRICS_STORE.lock() {
            if store.tokens_per_second.len() >= 100 {
                store.tokens_per_second.pop_front();
            }
            store.tokens_per_second.push_back(tps);
        }
    }

    pub fn record_tool_latency(tool: &str, ms: u64) {
        if let Ok(mut store) = METRICS_STORE.lock() {
            let latencies = store.tool_latencies.entry(tool.to_string()).or_default();
            if latencies.len() >= 50 {
                latencies.pop_front();
            }
            latencies.push_back(ms);
        }
    }

    pub fn record_error(category: &str) {
        if let Ok(mut store) = METRICS_STORE.lock() {
            *store.error_counts.entry(category.to_string()).or_insert(0) += 1;
        }
    }

    pub fn increment_requests(tokens: u64) {
        if let Ok(mut store) = METRICS_STORE.lock() {
            store.request_count += 1;
            store.total_tokens += tokens;
        }
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Returns all metrics as JSON: avg tokens/s, p50/p95 tool latencies per tool,
/// error rate, total requests/tokens.
#[tauri::command]
pub async fn get_metrics() -> Result<serde_json::Value, String> {
    let store = METRICS_STORE
        .lock()
        .map_err(|e| format!("Failed to lock metrics store: {}", e))?;

    // Average tokens per second
    let avg_tps = if store.tokens_per_second.is_empty() {
        0.0
    } else {
        let sum: f64 = store.tokens_per_second.iter().sum();
        sum / store.tokens_per_second.len() as f64
    };

    // Per-tool latency stats
    let mut tool_stats = serde_json::Map::new();
    for (tool, latencies) in &store.tool_latencies {
        let mut sorted: Vec<u64> = latencies.iter().copied().collect();
        sorted.sort_unstable();
        let p50 = percentile(&sorted, 50.0);
        let p95 = percentile(&sorted, 95.0);
        let avg = if sorted.is_empty() {
            0u64
        } else {
            sorted.iter().sum::<u64>() / sorted.len() as u64
        };
        tool_stats.insert(
            tool.clone(),
            serde_json::json!({
                "p50_ms": p50,
                "p95_ms": p95,
                "avg_ms": avg,
                "sample_count": sorted.len()
            }),
        );
    }

    // Error counts
    let error_map: serde_json::Map<String, serde_json::Value> = store
        .error_counts
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                serde_json::Value::Number(serde_json::Number::from(*v)),
            )
        })
        .collect();

    let total_errors: u64 = store.error_counts.values().sum();
    let error_rate = if store.request_count > 0 {
        total_errors as f64 / store.request_count as f64
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "avg_tokens_per_second": avg_tps,
        "tool_latencies": tool_stats,
        "error_counts": error_map,
        "error_rate": error_rate,
        "total_requests": store.request_count,
        "total_tokens": store.total_tokens,
        "tps_sample_count": store.tokens_per_second.len()
    }))
}

/// Reset all collected metrics.
#[tauri::command]
pub async fn reset_metrics() -> Result<(), String> {
    let mut store = METRICS_STORE
        .lock()
        .map_err(|e| format!("Failed to lock metrics store: {}", e))?;
    *store = MetricsData::new();
    Ok(())
}

/// Record a tool call metric (latency + success/failure).
#[tauri::command]
pub async fn record_tool_metric(
    tool_name: String,
    latency_ms: u64,
    success: bool,
) -> Result<(), String> {
    MetricsStore::record_tool_latency(&tool_name, latency_ms);
    if !success {
        MetricsStore::record_error(&format!("tool_failure:{}", tool_name));
    }
    Ok(())
}
