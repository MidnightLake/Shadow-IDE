use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Migration {
    pub id: String,
    pub name: String,
    pub status: String, // "pending", "applied", "failed"
    pub applied_at: Option<i64>,
    pub up_sql: String,
    pub down_sql: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MigrationResult {
    pub migration_id: String,
    pub success: bool,
    pub output: String,
}

/// List all migrations in <project>/migrations/ directory
#[tauri::command]
pub async fn list_migrations(project_path: String) -> Result<Vec<Migration>, String> {
    let mig_dir = Path::new(&project_path).join("migrations");
    if !mig_dir.exists() {
        std::fs::create_dir_all(&mig_dir).map_err(|e| e.to_string())?;
        return Ok(vec![]);
    }

    let mut migrations = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&mig_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("sql"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let content = std::fs::read_to_string(&path).unwrap_or_default();

        // Split on -- DOWN marker
        let (up_sql, down_sql) = if let Some(pos) = content.find("-- DOWN") {
            (
                content[..pos].trim().to_string(),
                content[pos + 7..].trim().to_string(),
            )
        } else {
            (content.trim().to_string(), String::new())
        };

        migrations.push(Migration {
            id: name.clone(),
            name: name.replace('_', " "),
            status: "pending".to_string(),
            applied_at: None,
            up_sql,
            down_sql,
        });
    }
    Ok(migrations)
}

/// Create a new migration file with AI-generated SQL
#[tauri::command]
pub async fn create_migration(
    project_path: String,
    name: String,
    description: String,
    api_key: Option<String>,
) -> Result<String, String> {
    let mig_dir = Path::new(&project_path).join("migrations");
    std::fs::create_dir_all(&mig_dir).map_err(|e| e.to_string())?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let filename = format!(
        "{}_{}.sql",
        timestamp,
        name.to_lowercase().replace(' ', "_")
    );
    let file_path = mig_dir.join(&filename);

    let sql_content = if let Some(key) = api_key.or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
    {
        // Generate SQL with AI
        let prompt = format!(
            "Generate a SQL migration for: {}. \
             Include an UP section with the migration SQL and a DOWN section (separated by '-- DOWN') \
             with the rollback SQL. Use standard SQL compatible with PostgreSQL. \
             Return ONLY the SQL, no explanation.",
            description
        );
        let resp = reqwest::Client::new()
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": "claude-haiku-4-5",
                "max_tokens": 1024,
                "messages": [{ "role": "user", "content": prompt }],
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| e.to_string())?;
        resp["content"][0]["text"]
            .as_str()
            .unwrap_or("-- TODO: add migration SQL\n\n-- DOWN\n-- TODO: add rollback SQL")
            .to_string()
    } else {
        format!("-- Migration: {}\n-- {}\n\n-- TODO: add migration SQL here\n\n-- DOWN\n-- TODO: add rollback SQL here", name, description)
    };

    std::fs::write(&file_path, sql_content).map_err(|e| e.to_string())?;
    Ok(file_path.to_string_lossy().to_string())
}

/// Run a migration against a DSN
#[tauri::command]
pub async fn run_migration(
    project_path: String,
    migration_id: String,
    dsn: String,
    direction: String, // "up" or "down"
) -> Result<MigrationResult, String> {
    let mig_dir = Path::new(&project_path).join("migrations");
    let files: Vec<_> = std::fs::read_dir(&mig_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&migration_id))
        .collect();

    let file = files
        .first()
        .ok_or(format!("Migration {} not found", migration_id))?;
    let content = std::fs::read_to_string(file.path()).map_err(|e| e.to_string())?;

    let sql = if direction == "down" {
        if let Some(pos) = content.find("-- DOWN") {
            content[pos + 7..].trim().to_string()
        } else {
            return Err("No DOWN section found in migration".to_string());
        }
    } else {
        if let Some(pos) = content.find("-- DOWN") {
            content[..pos].trim().to_string()
        } else {
            content.trim().to_string()
        }
    };

    // Write SQL to temp file and run via psql/sqlite3
    let tmp = std::env::temp_dir().join("shadow_migration.sql");
    std::fs::write(&tmp, &sql).map_err(|e| e.to_string())?;

    let out = if dsn.starts_with("postgres") {
        std::process::Command::new("psql")
            .args([&dsn, "-f", &tmp.to_string_lossy()])
            .output()
    } else {
        let path = dsn
            .trim_start_matches("sqlite://")
            .trim_start_matches("sqlite:");
        std::process::Command::new("sqlite3")
            .args([path, &format!(".read {}", tmp.display())])
            .output()
    }
    .map_err(|e| e.to_string())?;

    let _ = std::fs::remove_file(&tmp);

    Ok(MigrationResult {
        migration_id,
        success: out.status.success(),
        output: if out.status.success() {
            String::from_utf8_lossy(&out.stdout).to_string()
        } else {
            String::from_utf8_lossy(&out.stderr).to_string()
        },
    })
}
