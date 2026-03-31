/// Startup time measurement for Shadow IDE.
/// Tracks named timing marks from app boot to ready.

static STARTUP_MARKS: std::sync::LazyLock<std::sync::Mutex<Vec<(String, f64)>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// Record a named timing mark with a JS-style timestamp (milliseconds since epoch).
#[tauri::command]
pub async fn record_startup_mark(mark_name: String, timestamp_ms: f64) -> Result<(), String> {
    let mut marks = STARTUP_MARKS
        .lock()
        .map_err(|e| format!("Failed to lock startup marks: {}", e))?;
    marks.push((mark_name, timestamp_ms));
    Ok(())
}

/// Return all startup marks sorted by timestamp with inter-mark deltas.
#[tauri::command]
pub async fn get_startup_metrics() -> Result<serde_json::Value, String> {
    let marks = STARTUP_MARKS
        .lock()
        .map_err(|e| format!("Failed to lock startup marks: {}", e))?;

    let mut sorted = marks.clone();
    sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut entries = Vec::new();
    for (i, (name, ts)) in sorted.iter().enumerate() {
        let delta_ms = if i == 0 { 0.0 } else { ts - sorted[i - 1].1 };
        entries.push(serde_json::json!({
            "mark": name,
            "timestamp_ms": ts,
            "delta_ms": delta_ms
        }));
    }

    let total_ms = if sorted.len() >= 2 {
        sorted.last().map(|(_, t)| t).unwrap_or(&0.0)
            - sorted.first().map(|(_, t)| t).unwrap_or(&0.0)
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "marks": entries,
        "total_ms": total_ms,
        "mark_count": sorted.len()
    }))
}

/// Clear all startup marks.
#[tauri::command]
pub async fn clear_startup_metrics() -> Result<(), String> {
    let mut marks = STARTUP_MARKS
        .lock()
        .map_err(|e| format!("Failed to lock startup marks: {}", e))?;
    marks.clear();
    Ok(())
}
