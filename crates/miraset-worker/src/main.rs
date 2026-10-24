/// Miraset Worker Binary
use anyhow::Result;
use miraset_core::KeyPair;
use miraset_worker::{BackendType, Worker, WorkerConfig};
use std::sync::Arc;
use std::time::Duration;
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

/// Shared secret for verifying node dispatch authentication tags (H4).
/// Precedence:
/// 1. `MIRASET_DISPATCH_SECRET` env var (hex-encoded 32 bytes).
/// 2. A file at the path given by `MIRASET_DISPATCH_SECRET_FILE`.
fn dispatch_secret_from_env() -> Result<Option<[u8; 32]>> {
    if let Ok(hex_secret) = std::env::var("MIRASET_DISPATCH_SECRET") {
        let bytes = hex::decode(hex_secret.trim())?;
        if bytes.len() != 32 {
            anyhow::bail!("MIRASET_DISPATCH_SECRET must be 32 hex-encoded bytes");
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&bytes);
        return Ok(Some(secret));
    }

    if let Ok(path) = std::env::var("MIRASET_DISPATCH_SECRET_FILE") {
        let bytes = std::fs::read(path)?;
        if bytes.len() == 32 {
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            return Ok(Some(secret));
        }
        let encoded = std::str::from_utf8(&bytes)?;
        let decoded = hex::decode(encoded.trim())?;
        if decoded.len() != 32 {
            anyhow::bail!("dispatch secret file must contain 32 raw or hex-encoded bytes");
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&decoded);
        return Ok(Some(secret));
    }

    Ok(None)
}

fn bool_from_env(name: &str, default: bool) -> Result<bool> {
    match std::env::var(name) {
        Ok(value) if value.eq_ignore_ascii_case("true") || value == "1" => Ok(true),
        Ok(value) if value.eq_ignore_ascii_case("false") || value == "0" => Ok(false),
        Ok(value) => anyhow::bail!("{name} must be true, false, 1, or 0; got {value}"),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error.into()),
    }
}

fn u32_from_env(name: &str, default: u32) -> Result<u32> {
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|error| anyhow::anyhow!("invalid {name} value '{value}': {error}")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error.into()),
    }
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
    let allow_ollama_mock_fallback = bool_from_env("MIRASET_ALLOW_MOCK_FALLBACK", true)?;
    tracing::info!(
        "Ollama mock fallback: {}",
        if allow_ollama_mock_fallback {
            "enabled"
        } else {
            "disabled"
        }
    );

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

    let dispatch_secret = dispatch_secret_from_env()?;
    if dispatch_secret.is_some() {
        tracing::info!("Dispatch auth verification enabled");
    } else {
        tracing::warn!(
            "No MIRASET_DISPATCH_SECRET configured: worker will accept any /jobs/accept request (dev only)"
        );
    }

    let keypair = KeyPair::generate();
    let worker_id = blake3::hash(&bincode::serialize(&keypair.address())?).into();
    let bind_addr =
        std::env::var("MIRASET_WORKER_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let endpoint =
        std::env::var("MIRASET_WORKER_ENDPOINT").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let node_url =
        std::env::var("MIRASET_NODE_URL").unwrap_or_else(|_| "http://127.0.0.1:9944".to_string());
    let gpu_model = std::env::var("MIRASET_GPU_MODEL")
        .unwrap_or_else(|_| "Ollama host (hardware unspecified)".to_string());
    let vram_total_gib = u32_from_env("MIRASET_VRAM_TOTAL_GIB", 0)?;
    let config = WorkerConfig {
        worker_id,
        keypair,
        endpoint,
        node_url,
        backend_type,
        backend_url,
        backend_api_key,
        gpu_model,
        vram_total_gib,
        supported_models,
        dispatch_secret,
        ollama_mock_fallback: allow_ollama_mock_fallback,
        max_body_size: 2 * 1024 * 1024,
        request_timeout: Duration::from_secs(30),
        rate_limit_burst: 100,
        rate_limit_period: Duration::from_secs(1),
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

    let app = worker.router()?;

    tracing::info!("Worker listening on {}", bind_addr);
    tracing::info!("Connecting to node at {}", config.node_url);

    let listener = TcpListener::bind(&bind_addr).await?;
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
