/// Miraset Worker Binary
use anyhow::Result;
use miraset_core::KeyPair;
use miraset_worker::{BackendType, Worker, WorkerConfig};
use std::sync::Arc;
use tokio::net::TcpListener;

/// Resolve the inference backend type from `MIRASET_BACKEND_TYPE`
/// (default: `ollama`). Accepts: ollama, openai, llamacpp, tgi (case-insensitive).
fn backend_type_from_env() -> BackendType {
    match std::env::var("MIRASET_BACKEND_TYPE") {
        Ok(val) => BackendType::parse(&val).unwrap_or_else(|e| {
            tracing::warn!(
                "Invalid MIRASET_BACKEND_TYPE='{}': {}; defaulting to ollama",
                val,
                e
            );
            BackendType::default()
        }),
        Err(_) => BackendType::default(),
    }
}

/// Resolve the inference backend URL from `MIRASET_BACKEND_URL`.
/// Defaults to a per-backend default endpoint when unset. For
/// OpenAI-compatible family, the default depends on which preset the user
/// chose via `MIRASET_BACKEND_TYPE` (e.g. `lmstudio`, `vllm`, `localai`).
fn backend_url_from_env(backend_type: BackendType) -> String {
    match std::env::var("MIRASET_BACKEND_URL") {
        Ok(val) => val,
        Err(_) => default_backend_url(
            backend_type,
            std::env::var("MIRASET_BACKEND_TYPE").ok().as_deref(),
        ),
    }
}

/// Per-backend default URL. For `OpenAi` we also accept a friendly preset
/// name in `preset` (e.g. `lmstudio`, `vllm`, `localai`) so users don't have
/// to memorize the right port for every server.
fn default_backend_url(backend_type: BackendType, preset: Option<&str>) -> String {
    let preset = preset.map(|s| s.to_ascii_lowercase());
    let preset = preset.as_deref();
    match backend_type {
        BackendType::Ollama => "http://localhost:11434".to_string(),
        BackendType::OpenAi => match preset {
            Some("vllm") => "http://localhost:8000/v1".to_string(),
            Some("localai") | Some("local-ai") | Some("local_ai") => {
                "http://localhost:8080/v1".to_string()
            }
            // "lmstudio", "openai", "groq", "openrouter", "together", etc.
            _ => "http://localhost:1234/v1".to_string(),
        },
        BackendType::OpenLlm => "http://localhost:3000".to_string(),
        BackendType::LlamaCpp => "http://localhost:8080".to_string(),
        BackendType::Tgi => "http://localhost:8080".to_string(),
    }
}

/// Optional API key for cloud OpenAI-compatible providers (OpenAI, Groq,
/// OpenRouter, ...). Read from `MIRASET_BACKEND_API_KEY`. Ignored by
/// local backends that don't use auth.
fn backend_api_key_from_env() -> Option<String> {
    std::env::var("MIRASET_BACKEND_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Resolve the set of supported model ids.
///
/// Precedence:
/// 1. `MIRASET_SUPPORTED_MODELS` — comma-separated list (overrides everything).
/// 2. Per-backend defaults from `BackendType::default_models()`.
fn supported_models_from_env(backend_type: BackendType) -> Vec<String> {
    if let Ok(val) = std::env::var("MIRASET_SUPPORTED_MODELS") {
        let models: Vec<String> = val
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if !models.is_empty() {
            return models;
        }
    }
    backend_type
        .default_models()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let backend_type = backend_type_from_env();
    let backend_url = backend_url_from_env(backend_type);
    let backend_api_key = backend_api_key_from_env();
    let supported_models = supported_models_from_env(backend_type);

    tracing::info!(
        "Inference backend: {} ({}) at {}{}",
        backend_type.as_str(),
        backend_type.description(),
        backend_url,
        if backend_api_key.is_some() {
            " (auth enabled)"
        } else {
            ""
        }
    );
    tracing::info!(
        "Default models for {}: [{}]",
        backend_type.as_str(),
        supported_models.join(", ")
    );

    let config = WorkerConfig {
        worker_id: [1u8; 32],
        keypair: KeyPair::generate(),
        endpoint: "127.0.0.1:8080".to_string(),
        node_url: "http://127.0.0.1:9944".to_string(), // Node RPC port
        backend_type,
        backend_url,
        backend_api_key,
        gpu_model: "NVIDIA RTX 4090".to_string(),
        vram_total_gib: 24,
        supported_models,
    };

    let worker = Worker::new(config.clone());

    // Register worker on-chain
    let registered_worker_id = match worker.register_on_chain().await {
        Ok(worker_id) => {
            tracing::info!(
                "✓ Worker registered on-chain with ID: {}",
                hex::encode(worker_id)
            );
            Some(worker_id)
        }
        Err(e) => {
            tracing::warn!(
                "Failed to register on-chain (node may not be running): {}",
                e
            );
            None
        }
    };

    // D5: Start heartbeat loop (every 30 seconds)
    if let Some(worker_id) = registered_worker_id {
        tracing::info!("♥ Starting heartbeat loop (30s interval)");
        Arc::clone(&worker).start_heartbeat_loop(worker_id, 30);
    }

    let app = worker.router();

    tracing::info!("Worker listening on {}", config.endpoint);
    tracing::info!("Connecting to node at {}", config.node_url);

    let listener = TcpListener::bind(&config.endpoint).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_default_backend_url() {
        // Native backends
        assert_eq!(
            default_backend_url(BackendType::Ollama, None),
            "http://localhost:11434"
        );
        assert_eq!(
            default_backend_url(BackendType::LlamaCpp, None),
            "http://localhost:8080"
        );
        assert_eq!(
            default_backend_url(BackendType::Tgi, None),
            "http://localhost:8080"
        );
        assert_eq!(
            default_backend_url(BackendType::OpenLlm, None),
            "http://localhost:3000"
        );

        // OpenAI-compatible presets
        assert_eq!(
            default_backend_url(BackendType::OpenAi, Some("openai")),
            "http://localhost:1234/v1"
        );
        assert_eq!(
            default_backend_url(BackendType::OpenAi, Some("lmstudio")),
            "http://localhost:1234/v1"
        );
        assert_eq!(
            default_backend_url(BackendType::OpenAi, Some("vllm")),
            "http://localhost:8000/v1"
        );
        assert_eq!(
            default_backend_url(BackendType::OpenAi, Some("localai")),
            "http://localhost:8080/v1"
        );
        assert_eq!(
            default_backend_url(BackendType::OpenAi, Some("groq")),
            "http://localhost:1234/v1"
        );
        assert_eq!(
            default_backend_url(BackendType::OpenAi, None),
            "http://localhost:1234/v1"
        );
    }

    #[test]
    fn test_backend_type_parse_aliases() {
        // User-requested family members
        assert_eq!(BackendType::parse("lmstudio").unwrap(), BackendType::OpenAi);
        assert_eq!(BackendType::parse("vllm").unwrap(), BackendType::OpenAi);
        assert_eq!(BackendType::parse("localai").unwrap(), BackendType::OpenAi);
        assert_eq!(
            BackendType::parse("llamacpp").unwrap(),
            BackendType::LlamaCpp
        );
        // Cloud providers
        assert_eq!(BackendType::parse("groq").unwrap(), BackendType::OpenAi);
        assert_eq!(
            BackendType::parse("openrouter").unwrap(),
            BackendType::OpenAi
        );
        assert_eq!(BackendType::parse("together").unwrap(), BackendType::OpenAi);
        // OpenLLM
        assert_eq!(BackendType::parse("openllm").unwrap(), BackendType::OpenLlm);
        assert_eq!(BackendType::parse("bentoml").unwrap(), BackendType::OpenLlm);
    }
}
