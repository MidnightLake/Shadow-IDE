use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiAdapterConfig {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub project_slug: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub pipeline: Option<String>,
    #[serde(default)]
    pub job_path: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalCiRun {
    pub id: String,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub branch: String,
    pub sha: String,
    pub created_at: String,
    pub updated_at: String,
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalCiJob {
    pub id: String,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub stage: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub web_url: Option<String>,
}

pub struct CiAdapterState {
    file_path: PathBuf,
    configs: Mutex<Vec<CiAdapterConfig>>,
}

impl CiAdapterState {
    pub fn new(data_dir: PathBuf) -> Self {
        let file_path = data_dir.join("ci_adapters.json");
        let configs = std::fs::read(&file_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<CiAdapterConfig>>(&bytes).ok())
            .unwrap_or_default();
        Self {
            file_path,
            configs: Mutex::new(configs),
        }
    }

    fn save(&self) -> Result<(), String> {
        let configs = self
            .configs
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {}", e))?;
        }
        std::fs::write(
            &self.file_path,
            serde_json::to_vec_pretty(&*configs).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("Failed to write CI adapters: {}", e))
    }

    fn get_config(&self, id: &str) -> Result<CiAdapterConfig, String> {
        let configs = self
            .configs
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        configs
            .iter()
            .find(|cfg| cfg.id == id)
            .cloned()
            .ok_or_else(|| format!("CI adapter '{}' not found", id))
    }
}

fn value_status(raw: &str) -> (String, Option<String>) {
    let normalized = raw.to_lowercase();
    if matches!(normalized.as_str(), "running" | "in_progress" | "building") {
        return ("in_progress".to_string(), None);
    }
    if matches!(
        normalized.as_str(),
        "pending" | "queued" | "scheduled" | "created"
    ) {
        return ("queued".to_string(), None);
    }
    if matches!(normalized.as_str(), "success" | "passed" | "succeeded") {
        return ("completed".to_string(), Some("success".to_string()));
    }
    if matches!(normalized.as_str(), "failed" | "failure" | "error") {
        return ("completed".to_string(), Some("failure".to_string()));
    }
    if matches!(normalized.as_str(), "canceled" | "cancelled" | "aborted") {
        return ("completed".to_string(), Some("cancelled".to_string()));
    }
    ("completed".to_string(), None)
}

fn format_timestamp_millis(timestamp_ms: i64) -> String {
    timestamp_ms.to_string()
}

fn build_client(config: &CiAdapterConfig) -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(token) = config
        .token
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        match config.provider.as_str() {
            "circleci" => {
                headers.insert(
                    "Circle-Token",
                    reqwest::header::HeaderValue::from_str(token.trim())
                        .map_err(|e| e.to_string())?,
                );
            }
            _ => {
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token.trim()))
                        .map_err(|e| e.to_string())?,
                );
            }
        }
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

async fn fetch_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    config: &CiAdapterConfig,
) -> Result<serde_json::Value, String> {
    let mut req = client.request(method, url);
    if config.provider == "jenkins" {
        if let (Some(username), Some(token)) = (config.username.as_ref(), config.token.as_ref()) {
            req = req.basic_auth(username, Some(token));
        }
    }
    req.send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Request failed: {}", e))?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to decode response: {}", e))
}

async fn post_empty(
    client: &reqwest::Client,
    url: String,
    config: &CiAdapterConfig,
) -> Result<(), String> {
    let mut req = client.post(url);
    if config.provider == "jenkins" {
        if let (Some(username), Some(token)) = (config.username.as_ref(), config.token.as_ref()) {
            req = req.basic_auth(username, Some(token));
        }
    }
    req.send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Request failed: {}", e))?;
    Ok(())
}

async fn list_runs(config: &CiAdapterConfig) -> Result<Vec<ExternalCiRun>, String> {
    let client = build_client(config)?;
    match config.provider.as_str() {
        "circleci" => {
            let slug = config
                .project_slug
                .as_ref()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| "CircleCI adapter requires project_slug".to_string())?;
            let mut url = format!(
                "{}/project/{}/pipeline",
                config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://circleci.com/api/v2".to_string())
                    .trim_end_matches('/'),
                slug
            );
            if let Some(branch) = config.branch.as_ref().filter(|v| !v.trim().is_empty()) {
                url.push_str(&format!("?branch={}", branch));
            }
            let value = fetch_json(&client, reqwest::Method::GET, url, config).await?;
            let items = value["items"].as_array().cloned().unwrap_or_default();
            Ok(items
                .into_iter()
                .map(|item| ExternalCiRun {
                    id: item["id"].as_str().unwrap_or_default().to_string(),
                    name: format!("Pipeline #{}", item["number"].as_i64().unwrap_or_default()),
                    status: item["state"].as_str().unwrap_or("queued").to_string(),
                    conclusion: None,
                    branch: item["vcs"]["branch"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    sha: item["vcs"]["revision"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    created_at: item["created_at"].as_str().unwrap_or_default().to_string(),
                    updated_at: item["updated_at"].as_str().unwrap_or_default().to_string(),
                    web_url: item["web_url"].as_str().map(|v| v.to_string()),
                })
                .collect())
        }
        "buildkite" => {
            let org = config
                .organization
                .as_ref()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| "Buildkite adapter requires organization".to_string())?;
            let pipeline = config
                .pipeline
                .as_ref()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| "Buildkite adapter requires pipeline".to_string())?;
            let mut url = format!(
                "{}/organizations/{}/pipelines/{}/builds?per_page=20",
                config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.buildkite.com/v2".to_string())
                    .trim_end_matches('/'),
                org,
                pipeline
            );
            if let Some(branch) = config.branch.as_ref().filter(|v| !v.trim().is_empty()) {
                url.push_str(&format!("&branch={}", branch));
            }
            let items = fetch_json(&client, reqwest::Method::GET, url, config)
                .await?
                .as_array()
                .cloned()
                .unwrap_or_default();
            Ok(items
                .into_iter()
                .map(|item| {
                    let (status, conclusion) =
                        value_status(item["state"].as_str().unwrap_or("queued"));
                    ExternalCiRun {
                        id: item["number"].as_i64().unwrap_or_default().to_string(),
                        name: item["message"].as_str().unwrap_or("Build").to_string(),
                        status,
                        conclusion,
                        branch: item["branch"].as_str().unwrap_or_default().to_string(),
                        sha: item["commit"].as_str().unwrap_or_default().to_string(),
                        created_at: item["created_at"].as_str().unwrap_or_default().to_string(),
                        updated_at: item["finished_at"]
                            .as_str()
                            .or_else(|| item["created_at"].as_str())
                            .unwrap_or_default()
                            .to_string(),
                        web_url: item["web_url"].as_str().map(|v| v.to_string()),
                    }
                })
                .collect())
        }
        "jenkins" => {
            let base_url = config
                .base_url
                .as_ref()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| "Jenkins adapter requires base_url".to_string())?;
            let job_path = config
                .job_path
                .as_ref()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| "Jenkins adapter requires job_path".to_string())?;
            let normalized_path = job_path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(|segment| format!("job/{}", segment))
                .collect::<Vec<_>>()
                .join("/");
            let url = format!(
                "{}/{}/api/json?tree=builds[number,url,result,timestamp,building,id,displayName]",
                base_url.trim_end_matches('/'),
                normalized_path
            );
            let builds = fetch_json(&client, reqwest::Method::GET, url, config).await?["builds"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            Ok(builds
                .into_iter()
                .take(20)
                .map(|build| {
                    let (status, conclusion) = if build["building"].as_bool().unwrap_or(false) {
                        ("in_progress".to_string(), None)
                    } else {
                        value_status(build["result"].as_str().unwrap_or("completed").trim())
                    };
                    ExternalCiRun {
                        id: build["number"].as_i64().unwrap_or_default().to_string(),
                        name: build["displayName"].as_str().unwrap_or("Build").to_string(),
                        status,
                        conclusion,
                        branch: config.branch.clone().unwrap_or_default(),
                        sha: String::new(),
                        created_at: format_timestamp_millis(
                            build["timestamp"].as_i64().unwrap_or_default(),
                        ),
                        updated_at: format_timestamp_millis(
                            build["timestamp"].as_i64().unwrap_or_default(),
                        ),
                        web_url: build["url"].as_str().map(|v| v.to_string()),
                    }
                })
                .collect())
        }
        _ => Err("Unsupported CI provider".to_string()),
    }
}

async fn list_jobs(config: &CiAdapterConfig, run_id: &str) -> Result<Vec<ExternalCiJob>, String> {
    let client = build_client(config)?;
    match config.provider.as_str() {
        "circleci" => {
            let base = config
                .base_url
                .clone()
                .unwrap_or_else(|| "https://circleci.com/api/v2".to_string());
            let workflows_value = fetch_json(
                &client,
                reqwest::Method::GET,
                format!(
                    "{}/pipeline/{}/workflow",
                    base.trim_end_matches('/'),
                    run_id
                ),
                config,
            )
            .await?;
            let workflows = workflows_value["items"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut jobs = Vec::new();
            for workflow in workflows {
                let workflow_id = workflow["id"].as_str().unwrap_or_default();
                if workflow_id.is_empty() {
                    continue;
                }
                let workflow_name = workflow["name"].as_str().map(|v| v.to_string());
                let workflow_jobs = fetch_json(
                    &client,
                    reqwest::Method::GET,
                    format!(
                        "{}/workflow/{}/job",
                        base.trim_end_matches('/'),
                        workflow_id
                    ),
                    config,
                )
                .await?["items"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                jobs.extend(workflow_jobs.into_iter().map(|job| {
                    let (status, conclusion) =
                        value_status(job["status"].as_str().unwrap_or("queued"));
                    ExternalCiJob {
                        id: job["id"].as_str().unwrap_or_default().to_string(),
                        name: job["name"].as_str().unwrap_or("Job").to_string(),
                        status,
                        conclusion,
                        stage: workflow_name.clone(),
                        started_at: job["started_at"].as_str().map(|v| v.to_string()),
                        completed_at: job["stopped_at"].as_str().map(|v| v.to_string()),
                        web_url: job["web_url"].as_str().map(|v| v.to_string()),
                    }
                }));
            }
            Ok(jobs)
        }
        "buildkite" => {
            let org = config
                .organization
                .as_ref()
                .ok_or_else(|| "Buildkite adapter requires organization".to_string())?;
            let pipeline = config
                .pipeline
                .as_ref()
                .ok_or_else(|| "Buildkite adapter requires pipeline".to_string())?;
            let base = config
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.buildkite.com/v2".to_string());
            let jobs = fetch_json(
                &client,
                reqwest::Method::GET,
                format!(
                    "{}/organizations/{}/pipelines/{}/builds/{}",
                    base.trim_end_matches('/'),
                    org,
                    pipeline,
                    run_id
                ),
                config,
            )
            .await?["jobs"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            Ok(jobs
                .into_iter()
                .filter(|job| job["type"].as_str().unwrap_or("") == "script")
                .map(|job| {
                    let (status, conclusion) =
                        value_status(job["state"].as_str().unwrap_or("queued"));
                    ExternalCiJob {
                        id: job["id"].as_str().unwrap_or_default().to_string(),
                        name: job["name"].as_str().unwrap_or("Job").to_string(),
                        status,
                        conclusion,
                        stage: job["step_key"].as_str().map(|v| v.to_string()),
                        started_at: job["started_at"].as_str().map(|v| v.to_string()),
                        completed_at: job["finished_at"].as_str().map(|v| v.to_string()),
                        web_url: job["web_url"].as_str().map(|v| v.to_string()),
                    }
                })
                .collect())
        }
        "jenkins" => {
            let base_url = config
                .base_url
                .as_ref()
                .ok_or_else(|| "Jenkins adapter requires base_url".to_string())?;
            let job_path = config
                .job_path
                .as_ref()
                .ok_or_else(|| "Jenkins adapter requires job_path".to_string())?;
            let normalized_path = job_path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(|segment| format!("job/{}", segment))
                .collect::<Vec<_>>()
                .join("/");
            let pipeline_url = format!(
                "{}/{}/{}/wfapi/describe",
                base_url.trim_end_matches('/'),
                normalized_path,
                run_id
            );
            let value = fetch_json(&client, reqwest::Method::GET, pipeline_url, config)
                .await
                .unwrap_or_else(|_| serde_json::json!({}));
            let stages = value["stages"].as_array().cloned().unwrap_or_default();
            if stages.is_empty() {
                return Ok(vec![ExternalCiJob {
                    id: run_id.to_string(),
                    name: "Jenkins Build".to_string(),
                    status: "completed".to_string(),
                    conclusion: None,
                    stage: None,
                    started_at: None,
                    completed_at: None,
                    web_url: None,
                }]);
            }
            Ok(stages
                .into_iter()
                .map(|stage| {
                    let (status, conclusion) =
                        value_status(stage["status"].as_str().unwrap_or("queued"));
                    ExternalCiJob {
                        id: stage["id"].as_str().unwrap_or_default().to_string(),
                        name: stage["name"].as_str().unwrap_or("Stage").to_string(),
                        status,
                        conclusion,
                        stage: None,
                        started_at: stage["startTimeMillis"]
                            .as_i64()
                            .map(format_timestamp_millis),
                        completed_at: stage["durationMillis"].as_i64().map(|duration| {
                            format_timestamp_millis(
                                stage["startTimeMillis"].as_i64().unwrap_or_default() + duration,
                            )
                        }),
                        web_url: None,
                    }
                })
                .collect())
        }
        _ => Err("Unsupported CI provider".to_string()),
    }
}

async fn rerun(config: &CiAdapterConfig, run_id: &str) -> Result<(), String> {
    let client = build_client(config)?;
    match config.provider.as_str() {
        "circleci" => {
            let base = config
                .base_url
                .clone()
                .unwrap_or_else(|| "https://circleci.com/api/v2".to_string());
            let workflows = fetch_json(
                &client,
                reqwest::Method::GET,
                format!(
                    "{}/pipeline/{}/workflow",
                    base.trim_end_matches('/'),
                    run_id
                ),
                config,
            )
            .await?["items"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let workflow_id = workflows
                .first()
                .and_then(|workflow| workflow["id"].as_str())
                .ok_or_else(|| "No workflow found for CircleCI pipeline".to_string())?;
            post_empty(
                &client,
                format!(
                    "{}/workflow/{}/rerun",
                    base.trim_end_matches('/'),
                    workflow_id
                ),
                config,
            )
            .await
        }
        "buildkite" => {
            let org = config
                .organization
                .as_ref()
                .ok_or_else(|| "Buildkite adapter requires organization".to_string())?;
            let pipeline = config
                .pipeline
                .as_ref()
                .ok_or_else(|| "Buildkite adapter requires pipeline".to_string())?;
            let base = config
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.buildkite.com/v2".to_string());
            post_empty(
                &client,
                format!(
                    "{}/organizations/{}/pipelines/{}/builds/{}/rebuild",
                    base.trim_end_matches('/'),
                    org,
                    pipeline,
                    run_id
                ),
                config,
            )
            .await
        }
        "jenkins" => {
            let base_url = config
                .base_url
                .as_ref()
                .ok_or_else(|| "Jenkins adapter requires base_url".to_string())?;
            let job_path = config
                .job_path
                .as_ref()
                .ok_or_else(|| "Jenkins adapter requires job_path".to_string())?;
            let normalized_path = job_path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(|segment| format!("job/{}", segment))
                .collect::<Vec<_>>()
                .join("/");
            post_empty(
                &client,
                format!(
                    "{}/{}/build",
                    base_url.trim_end_matches('/'),
                    normalized_path
                ),
                config,
            )
            .await
        }
        _ => Err("Unsupported CI provider".to_string()),
    }
}

async fn fetch_log(config: &CiAdapterConfig, run_id: &str) -> Result<String, String> {
    let client = build_client(config)?;
    match config.provider.as_str() {
        "jenkins" => {
            let base_url = config
                .base_url
                .as_ref()
                .ok_or_else(|| "Jenkins adapter requires base_url".to_string())?;
            let job_path = config
                .job_path
                .as_ref()
                .ok_or_else(|| "Jenkins adapter requires job_path".to_string())?;
            let normalized_path = job_path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(|segment| format!("job/{}", segment))
                .collect::<Vec<_>>()
                .join("/");
            let mut req = client.get(format!(
                "{}/{}/{}/consoleText",
                base_url.trim_end_matches('/'),
                normalized_path,
                run_id
            ));
            if let (Some(username), Some(token)) = (config.username.as_ref(), config.token.as_ref())
            {
                req = req.basic_auth(username, Some(token));
            }
            req.send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?
                .error_for_status()
                .map_err(|e| format!("Request failed: {}", e))?
                .text()
                .await
                .map_err(|e| format!("Failed to decode log: {}", e))
        }
        _ => Err("Raw logs are only supported for Jenkins in the generic adapter".to_string()),
    }
}

#[tauri::command]
pub fn ci_adapter_list_configs(
    state: tauri::State<'_, CiAdapterState>,
) -> Result<Vec<CiAdapterConfig>, String> {
    Ok(state
        .configs
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .clone())
}

#[tauri::command]
pub fn ci_adapter_save_config(
    config: CiAdapterConfig,
    state: tauri::State<'_, CiAdapterState>,
) -> Result<CiAdapterConfig, String> {
    let mut config = config;
    if config.id.trim().is_empty() {
        config.id = uuid::Uuid::new_v4().to_string();
    }
    let mut configs = state
        .configs
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    if let Some(existing) = configs.iter_mut().find(|existing| existing.id == config.id) {
        *existing = config.clone();
    } else {
        configs.push(config.clone());
    }
    drop(configs);
    state.save()?;
    Ok(config)
}

#[tauri::command]
pub fn ci_adapter_delete_config(
    id: String,
    state: tauri::State<'_, CiAdapterState>,
) -> Result<(), String> {
    let mut configs = state
        .configs
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    configs.retain(|config| config.id != id);
    drop(configs);
    state.save()
}

#[tauri::command]
pub async fn ci_adapter_list_runs(
    id: String,
    state: tauri::State<'_, CiAdapterState>,
) -> Result<Vec<ExternalCiRun>, String> {
    let config = state.get_config(&id)?;
    list_runs(&config).await
}

#[tauri::command]
pub async fn ci_adapter_list_jobs(
    id: String,
    run_id: String,
    state: tauri::State<'_, CiAdapterState>,
) -> Result<Vec<ExternalCiJob>, String> {
    let config = state.get_config(&id)?;
    list_jobs(&config, &run_id).await
}

#[tauri::command]
pub async fn ci_adapter_rerun(
    id: String,
    run_id: String,
    state: tauri::State<'_, CiAdapterState>,
) -> Result<(), String> {
    let config = state.get_config(&id)?;
    rerun(&config, &run_id).await
}

#[tauri::command]
pub async fn ci_adapter_fetch_log(
    id: String,
    run_id: String,
    state: tauri::State<'_, CiAdapterState>,
) -> Result<String, String> {
    let config = state.get_config(&id)?;
    fetch_log(&config, &run_id).await
}
