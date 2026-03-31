mod agent_queue;
mod agent_runner;
mod ai_bridge;
mod ai_macros;
mod ai_skills;
mod bluetooth_server;
mod ci_adapter;
mod cloud_sync;
mod collaboration;
pub mod config;
mod dap_client;
mod diagnostics;
pub mod errors;
mod feature_flags;
mod ferrum_bridge;
mod fs_commands;
mod gamedev;
mod git_commands;
mod llm_loader;
pub mod llm_provider;
mod lsp_client;
mod manager;
mod metrics;
mod migrations;
mod model_scanner;
mod pairing;
pub mod platform;
mod plugin_manager;
mod project_manager;
mod rag_index;
mod remote_server;
mod session;
mod session_bridge;
mod shell_commands;
mod startup;
mod terminal;
mod todo_scanner;
pub mod token_budget;
mod token_optimizer;
mod tool_calling;

use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

/// Install the bundled shadowai CLI binary to user's PATH.
/// On Windows: copies to %USERPROFILE%\.cargo\bin\ (cargo's bin dir is usually in PATH).
/// On Linux/macOS: copies to ~/.local/bin/ (commonly in PATH).
fn install_cli(app: &tauri::AppHandle) {
    let binary_name = if cfg!(windows) {
        "shadowai.exe"
    } else {
        "shadowai"
    };

    // Find the bundled CLI in our resource directory
    let resource_path = app.path().resource_dir().ok().map(|d| d.join(binary_name));
    let source = match resource_path {
        Some(p) if p.exists() => p,
        _ => return, // Not bundled or dev mode
    };

    // Determine install destination
    let dest_dir = if cfg!(windows) {
        dirs_next::home_dir().map(|h| h.join(".cargo").join("bin"))
    } else {
        dirs_next::home_dir().map(|h| h.join(".local").join("bin"))
    };
    let dest_dir = match dest_dir {
        Some(d) => d,
        None => return,
    };
    let dest = dest_dir.join(binary_name);

    // Skip if same version already installed (compare file size as simple check)
    if dest.exists() {
        let src_size = std::fs::metadata(&source).map(|m| m.len()).unwrap_or(0);
        let dst_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        if src_size == dst_size {
            return; // Already up to date
        }
    }

    let _ = std::fs::create_dir_all(&dest_dir);
    if let Err(e) = std::fs::copy(&source, &dest) {
        log::warn!("Failed to install shadowai CLI: {}", e);
        return;
    }

    // On Unix, make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }

    log::info!("Installed shadowai CLI to {}", dest.display());

    // Auto-configure a localhost token so the CLI works immediately
    if let Some(config_dir) = dirs_next::config_dir() {
        let shadow_dir = config_dir.join("shadowai");
        let token_file = shadow_dir.join("token");
        if !token_file.exists() {
            let _ = std::fs::create_dir_all(&shadow_dir);
            let _ = std::fs::write(&token_file, "shadowide-local");
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let terminal_manager = Arc::new(terminal::TerminalManager::new());
    let ai_config = ai_bridge::AiConfig::new();
    let token_cache = token_optimizer::TokenCache::new();
    let warm_cache = token_optimizer::WarmCache::new();
    let token_settings = token_optimizer::TokenSettings::new();
    let ferrum_state = match ferrum_bridge::FerrumState::new() {
        Ok(state) => state,
        Err(e) => {
            log::error!(
                "Failed to init Ferrum state: {}. Chat features will be unavailable.",
                e
            );
            eprintln!(
                "[shadow-ide] Failed to init Ferrum state: {}. Chat features will be unavailable.",
                e
            );
            ferrum_bridge::FerrumState::empty()
        }
    };

    // Set up data directory for pairing/certs/projects
    let data_dir = dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("shadow-ide");
    let pairing_manager = Arc::new(pairing::PairingManager::new(data_dir.clone()));
    let remote_state = Arc::new(remote_server::RemoteServerState::new(pairing_manager));
    let plugin_state = plugin_manager::PluginManagerState::new(data_dir.clone());
    let cloud_sync_state = cloud_sync::CloudSyncState::new(data_dir.clone());
    let collaboration_state = collaboration::CollaborationState::new();
    let ci_adapter_state = ci_adapter::CiAdapterState::new(data_dir.clone());
    let feature_flags_state = feature_flags::FeatureFlagsState::new(data_dir.clone());
    let project_state = project_manager::ProjectManagerState::new(data_dir);
    let lsp_state = lsp_client::LspState::new();
    let rag_state = Arc::new(rag_index::RagState::new());
    let llm_server_state = llm_loader::LlmServerState::new();
    let bt_state = bluetooth_server::BluetoothState::new();
    let watcher_state = fs_commands::WatcherState::new();
    let shadow_runtime_state = gamedev::ShadowRuntimeState::default();
    let shadow_config = Arc::new(std::sync::Mutex::new(config::ShadowConfig::default()));
    let provider_registry = Arc::new(std::sync::Mutex::new(llm_provider::ProviderRegistry::new()));
    let session_manager = Arc::new(manager::SessionManager::new());
    let primary_session = session_bridge::PrimarySession::new();
    let macro_state = Arc::new(ai_macros::MacroState::new());
    macro_state.load();

    tauri::Builder::default()
        .manage(terminal_manager)
        .manage(ai_config)
        .manage(token_cache)
        .manage(warm_cache)
        .manage(token_settings)
        .manage(ferrum_state)
        .manage(remote_state)
        .manage(plugin_state)
        .manage(cloud_sync_state)
        .manage(collaboration_state)
        .manage(ci_adapter_state)
        .manage(feature_flags_state)
        .manage(project_state)
        .manage(lsp_state)
        .manage(rag_state)
        .manage(llm_server_state)
        .manage(bt_state)
        .manage(watcher_state)
        .manage(shadow_runtime_state)
        .manage(shadow_config)
        .manage(provider_registry)
        .manage(session_manager)
        .manage(primary_session)
        .manage(macro_state)
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        // updater plugin disabled until signing keypair is generated
        // .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Always show the desktop window — PC is the primary device.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }

            // Install rustls CryptoProvider before TLS is used
            let _ = rustls::crypto::ring::default_provider().install_default();

            // Auto-install the shadowai CLI to user's PATH
            install_cli(app.handle());

            // Remote server is started manually via remote_start_server command.

            // Start config hot-reload watcher
            let config_path = dirs_next::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("shadow-ide")
                .join("shadow-ide.toml");
            config::start_config_watcher(app.handle().clone(), config_path);
            collaboration::install_event_bridge(app.handle());

            // Session cleanup task — removes idle sessions every 5 minutes
            let cleanup_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                loop {
                    interval.tick().await;
                    let mgr = cleanup_handle.state::<Arc<manager::SessionManager>>();
                    for id in mgr.expired_ids() {
                        mgr.remove(&id);
                        log::info!("Session {} expired and removed", id);
                    }
                    log::debug!("Active sessions: {}", mgr.count());
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // File system
            fs_commands::read_directory,
            fs_commands::read_file_content,
            fs_commands::write_file_content,
            fs_commands::get_home_dir,
            fs_commands::create_directory,
            fs_commands::delete_entry,
            fs_commands::rename_entry,
            fs_commands::get_file_info,
            fs_commands::read_file_chunk,
            // Terminal
            terminal::create_terminal,
            terminal::write_terminal,
            terminal::resize_terminal,
            terminal::close_terminal,
            terminal::terminal_share_start,
            terminal::terminal_share_stop,
            terminal::terminal_share_status,
            terminal::terminal_share_write,
            // AI Bridge
            ai_bridge::ai_set_base_url,
            ai_bridge::ai_get_models,
            ai_bridge::ai_check_connection,
            ai_bridge::ai_detect_providers,
            ai_bridge::ai_profile_model,
            ai_bridge::ai_chat_stream,
            ai_bridge::abort_ai_chat,
            ai_bridge::ai_chat_with_tools,
            ai_bridge::ai_complete_code,
            ai_bridge::ai_explain_error,
            ai_bridge::ai_list_memories,
            ai_bridge::ai_delete_memory,
            ai_bridge::ai_regenerate_memory,
            config::config_load,
            config::config_save,
            config::config_get,
            config::config_generate_default,
            // FerrumChat
            ferrum_bridge::ferrum_get_config,
            ferrum_bridge::ferrum_save_config,
            ferrum_bridge::ferrum_get_profiles,
            ferrum_bridge::ferrum_add_profile,
            ferrum_bridge::ferrum_remove_profile,
            ferrum_bridge::ferrum_set_active_profile,
            ferrum_bridge::ferrum_get_active_profile,
            ferrum_bridge::ferrum_list_sessions,
            ferrum_bridge::ferrum_create_session,
            ferrum_bridge::ferrum_load_messages,
            ferrum_bridge::ferrum_save_message,
            ferrum_bridge::ferrum_delete_session,
            ferrum_bridge::ferrum_rename_session,
            ferrum_bridge::ferrum_pin_session,
            ferrum_bridge::ferrum_get_latest_session,
            ferrum_bridge::ferrum_get_session_token_count,
            ferrum_bridge::ferrum_export_session,
            ferrum_bridge::ferrum_check_compaction,
            ferrum_bridge::ferrum_get_compaction_prompt,
            ferrum_bridge::ferrum_apply_compaction,
            ferrum_bridge::ferrum_check_provider,
            ferrum_bridge::ferrum_list_provider_models,
            // Token Optimizer
            token_optimizer::token_get_cache_stats,
            token_optimizer::token_clear_cache,
            token_optimizer::token_update_settings,
            token_optimizer::token_set_cache_ttl,
            token_optimizer::token_set_clean_mode,
            token_optimizer::token_set_max_context,
            token_optimizer::token_count_text,
            // Warm Cache
            token_optimizer::warm_cache_get,
            token_optimizer::warm_cache_stats,
            token_optimizer::warm_cache_clear,
            token_optimizer::warm_cache_evict,
            token_optimizer::warm_cache_semantic_search,
            // TODO Scanner
            todo_scanner::scan_todos,
            todo_scanner::scan_file_todos,
            // Project Manager
            project_manager::project_open,
            project_manager::project_list_recent,
            project_manager::project_remove_recent,
            project_manager::project_save_state,
            project_manager::project_load_state,
            project_manager::project_clear_recent,
            shell_commands::shell_exec,
            // Cloud sync
            cloud_sync::cloud_list_snippets,
            cloud_sync::cloud_save_snippet,
            cloud_sync::cloud_delete_snippet,
            cloud_sync::cloud_get_bundle_status,
            cloud_sync::cloud_export_bundle,
            cloud_sync::cloud_import_bundle,
            collaboration::collab_get_snapshot,
            ci_adapter::ci_adapter_list_configs,
            ci_adapter::ci_adapter_save_config,
            ci_adapter::ci_adapter_delete_config,
            ci_adapter::ci_adapter_list_runs,
            ci_adapter::ci_adapter_list_jobs,
            ci_adapter::ci_adapter_rerun,
            ci_adapter::ci_adapter_fetch_log,
            feature_flags::feature_flag_list_configs,
            feature_flags::feature_flag_save_config,
            feature_flags::feature_flag_delete_config,
            feature_flags::feature_flag_list_flags,
            feature_flags::feature_flag_set_enabled,
            // Plugins
            plugin_manager::plugin_api_info,
            plugin_manager::plugin_list,
            plugin_manager::plugin_reload,
            plugin_manager::plugin_install,
            plugin_manager::plugin_update,
            plugin_manager::plugin_grant_permissions,
            plugin_manager::plugin_revoke_permissions,
            plugin_manager::plugin_uninstall,
            plugin_manager::plugin_enable,
            plugin_manager::plugin_disable,
            // Remote Server
            remote_server::remote_get_info,
            remote_server::remote_generate_cert,
            remote_server::remote_get_qr_code,
            remote_server::remote_list_devices,
            remote_server::remote_remove_device,
            remote_server::remote_update_device_permissions,
            remote_server::remote_start_server,
            remote_server::remote_stop_server,
            remote_server::remote_set_timeout,
            remote_server::remote_check_cert_expiry,
            remote_server::remote_update_state,
            remote_server::remote_detect_network,
            remote_server::remote_get_noise_pubkey,
            remote_server::remote_get_recording_status,
            remote_server::remote_start_recording,
            remote_server::remote_stop_recording,
            remote_server::remote_list_recordings,
            remote_server::remote_load_recording,
            // Git
            git_commands::git_is_repo,
            git_commands::git_status,
            git_commands::git_file_diff,
            git_commands::git_file_blame,
            git_commands::git_worktree_list,
            git_commands::git_worktree_add,
            git_commands::git_worktree_remove,
            git_commands::git_stash_list,
            git_commands::git_stash_show,
            git_commands::git_cherry_pick,
            git_commands::git_commit_details,
            git_commands::git_branch_graph,
            // Terminal (new)
            terminal::detect_shell,
            terminal::list_terminals,
            // File system (new)
            fs_commands::search_files_by_name,
            fs_commands::watch_workspace,
            fs_commands::unwatch_workspace,
            fs_commands::search_in_files,
            fs_commands::replace_in_files,
            fs_commands::create_file_with_template,
            // Project Manager (new)
            project_manager::project_load_config,
            project_manager::project_save_config,
            // LSP
            lsp_client::lsp_detect_servers,
            lsp_client::lsp_start,
            lsp_client::lsp_stop,
            lsp_client::lsp_stop_all,
            lsp_client::lsp_did_open,
            lsp_client::lsp_did_change,
            lsp_client::lsp_did_save,
            lsp_client::lsp_did_close,
            lsp_client::lsp_hover,
            lsp_client::lsp_completion,
            lsp_client::lsp_goto_definition,
            lsp_client::auto_install_lsp,
            lsp_client::detect_project_lsp_servers,
            lsp_client::lsp_code_action,
            lsp_client::lsp_rename,
            lsp_client::lsp_references,
            lsp_client::lsp_inlay_hints,
            lsp_client::lsp_call_hierarchy_incoming,
            // RAG
            rag_index::rag_build_index,
            rag_index::rag_query,
            rag_index::rag_query_structured,
            rag_index::rag_get_stats,
            rag_index::rag_index_documents,
            rag_index::ensure_documents_folder,
            rag_index::rag_list_documents,
            rag_index::rag_auto_clean,
            rag_index::rag_configure_embeddings,
            rag_index::rag_get_embedding_config,
            rag_index::rag_embed_chunks,
            rag_index::rag_semantic_search,
            rag_index::rag_embedding_stats,
            rag_index::rag_watch_start,
            rag_index::rag_watch_stop,
            rag_index::rag_watch_status,
            // Model Scanner
            model_scanner::scan_local_models,
            // LLM Loader
            llm_loader::detect_hardware,
            llm_loader::auto_configure_llm,
            llm_loader::download_hf_model,
            llm_loader::search_hf_models,
            llm_loader::list_hf_repo_files,
            llm_loader::delete_local_model,
            // LLM Engine
            llm_loader::check_engine,
            llm_loader::install_engine,
            llm_loader::uninstall_engine,
            llm_loader::detect_recommended_backend,
            llm_loader::recommend_model_for_task,
            llm_loader::list_installed_engines,
            llm_loader::list_engine_files,
            // LLM Server
            llm_loader::launch_llm_server,
            llm_loader::stop_llm_server,
            llm_loader::unload_llm_model,
            llm_loader::get_llm_server_status,
            llm_loader::get_llm_network_info,
            llm_loader::get_gpu_memory_stats,
            // Provider
            llm_provider::provider_list,
            llm_provider::provider_get_active,
            llm_provider::provider_set_active,
            llm_provider::provider_add,
            llm_provider::provider_infer,
            llm_provider::provider_detect_available,
            llm_provider::detect_ollama,
            llm_provider::list_ollama_models,
            llm_provider::ollama_pull_model,
            llm_provider::get_default_routing_rules,
            // Bluetooth
            bluetooth_server::bt_start_server,
            bluetooth_server::bt_stop_server,
            bluetooth_server::bt_get_status,
            bluetooth_server::bt_get_pairing_qr,
            // Session Bridge (PC ↔ phone unified chat)
            session_bridge::session_join,
            session_bridge::session_chat,
            session_bridge::session_info,
            session_bridge::session_replay,
            session_bridge::session_abort,
            // AI Macros (automation triggers)
            ai_macros::macro_list,
            ai_macros::macro_add,
            ai_macros::macro_update,
            ai_macros::macro_delete,
            ai_macros::macro_trigger,
            ai_macros::macro_load_presets,
            ai_skills::ai_classify_skill,
            ai_skills::ai_skill_config,
            ai_skills::ai_skill_prompt,
            // Agent runner
            agent_runner::agent_pause,
            agent_runner::agent_resume,
            agent_runner::agent_is_paused,
            agent_runner::list_agent_templates,
            // Spend stats
            ai_bridge::get_spend_stats,
            // Diagnostics
            diagnostics::analyze_complexity,
            diagnostics::find_duplicates,
            diagnostics::scan_security,
            diagnostics::scan_licenses,
            diagnostics::find_dead_code,
            diagnostics::run_cpu_profiler,
            diagnostics::run_memory_profiler,
            // Game Development
            gamedev::connect_godot_lsp,
            gamedev::run_godot_project,
            gamedev::get_shader_snippets,
            gamedev::optimize_texture,
            gamedev::list_godot_assets,
            // Security
            tool_calling::security::scan_file_for_secrets,
            // DAP (Debug Adapter Protocol)
            dap_client::dap_launch,
            dap_client::dap_set_breakpoints,
            dap_client::dap_continue,
            dap_client::dap_step_over,
            dap_client::dap_step_into,
            dap_client::dap_step_out,
            dap_client::dap_pause,
            dap_client::dap_stop,
            dap_client::dap_list_adapters,
            // Git (new)
            git_commands::git_blame,
            git_commands::git_diff_hunks,
            git_commands::git_stage_hunk,
            git_commands::git_unstage_hunk,
            // LLM Provider (new)
            llm_provider::get_model_metadata,
            llm_provider::detect_lm_studio,
            llm_provider::list_lm_studio_models,
            llm_provider::detect_vllm,
            llm_provider::list_vllm_models,
            llm_provider::start_llama_server,
            llm_provider::stop_llama_server,
            llm_provider::download_gguf_model,
            llm_provider::benchmark_model,
            // Diagnostics (new)
            diagnostics::analyze_bundle,
            // AI Bridge (new)
            ai_bridge::chat::ai_generate_docstring,
            ai_bridge::chat::ai_generate_readme,
            ai_bridge::chat::ai_generate_architecture_diagram,
            ai_bridge::chat::ai_sql_query,
            // AI debug + conversation state
            ai_bridge::chat::ai_debug_explain,
            ai_bridge::chat::get_conversation_state,
            // LSP type hierarchy
            lsp_client::lsp_type_hierarchy_supertypes,
            lsp_client::lsp_type_hierarchy_subtypes,
            // LLM provider fallback
            llm_provider::model_with_fallback,
            // Migrations
            migrations::list_migrations,
            migrations::create_migration,
            migrations::run_migration,
            // Git PR / CI integration
            git_commands::git_list_prs,
            git_commands::git_create_pr,
            git_commands::git_get_pr,
            git_commands::git_ci_status,
            // Game dev
            gamedev::connect_godot_debugger,
            gamedev::validate_wgsl,
            gamedev::parse_tscn_preview,
            // ShadowEditor project
            gamedev::shadow_get_project_info,
            gamedev::shadow_parse_scene,
            gamedev::shadow_validate_scene,
            gamedev::shadow_scene_add_entity,
            gamedev::shadow_scene_remove_entity,
            gamedev::shadow_scene_set_entity_name,
            gamedev::shadow_scene_add_component,
            gamedev::shadow_scene_remove_component,
            gamedev::shadow_scene_set_component_field,
            gamedev::shadow_get_ai_context,
            gamedev::shadow_trigger_build,
            gamedev::shadow_runtime_status,
            gamedev::shadow_runtime_load,
            gamedev::shadow_runtime_step,
            gamedev::shadow_runtime_stop,
            gamedev::shadow_runtime_save_scene,
            gamedev::shadow_runtime_capture_scene,
            gamedev::shadow_load_planengine_docs,
            gamedev::shadow_list_assets,
            gamedev::shadow_import_assets,
            gamedev::shadow_list_source_files,
            gamedev::shadow_run_header_tool,
            gamedev::shadow_load_reflection,
            gamedev::shadow_list_templates,
            gamedev::shadow_new_project,
            gamedev::shadow_ai_history_load,
            gamedev::shadow_ai_history_append,
            gamedev::shadow_ai_history_clear,
            gamedev::shadow_generate_compile_commands,
            gamedev::shadow_get_last_build_log,
            gamedev::shadow_inspector_suggestions,
            gamedev::shadow_launch_native_editor,
            // DB query analysis
            diagnostics::analyze_db_queries,
            // Snapshot testing
            diagnostics::run_snapshot_tests,
            diagnostics::update_snapshots,
            // Mutation testing
            diagnostics::run_mutation_tests,
            // API docs export
            diagnostics::export_api_docs,
            // Metrics
            metrics::get_metrics,
            metrics::reset_metrics,
            metrics::record_tool_metric,
            // Startup timing
            startup::record_startup_mark,
            startup::get_startup_metrics,
            startup::clear_startup_metrics,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
