/// Inference backend abstraction
///
/// Supports multiple backends:
/// - `OllamaBackend` (local Ollama, native `/api/generate` + `/api/tags`)
/// - `OpenAiCompatibleBackend` (any OpenAI-compatible endpoint: LM Studio, vLLM,
///   LocalAI, llama.cpp OpenAI shim, TGI OpenAI shim, and cloud providers like
///   OpenAI, Groq, OpenRouter, Together — requires `api_key` for cloud)
/// - `LlamaCppBackend` (native llama.cpp server `/completion` + `/props`)
/// - `TgiBackend` (HuggingFace text-generation-inference `/generate` + `/info`)
/// - `MockBackend` (deterministic testing backend)
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

mod llamacpp;
mod mock;
mod ollama;
mod openai;
mod openllm;
mod tgi;

pub use llamacpp::LlamaCppBackend;
pub use mock::MockBackend;
pub use ollama::OllamaBackend;
pub use openai::OpenAiCompatibleBackend;
pub use openllm::OpenLlmBackend;
pub use tgi::TgiBackend;

/// Inference backend trait
#[async_trait]
pub trait InferenceBackend: Send + Sync {
    /// Generate text completion
    async fn generate(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u64,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<GenerationResponse>;

    /// Check if model is loaded
    async fn is_model_loaded(&self, model: &str) -> Result<bool>;

    /// List available models
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
}

/// Generation response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationResponse {
    pub text: String,
    pub tokens: Vec<String>,
    pub token_count: u64,
    pub model: String,
}

/// Model information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub size: u64,
    pub family: String,
}

/// Backend type selector (parsed from `MIRASET_BACKEND_TYPE` env var).
///
/// Variants:
/// - `Ollama` — value `ollama` (default)
/// - `OpenAi` — value `openai` (OpenAI-compatible: LM Studio, vLLM, OpenAI, Groq, ...)
/// - `OpenLlm` — value `openllm` (BentoML OpenLLM native `/v1/generate`)
/// - `LlamaCpp` — value `llamacpp` (native llama.cpp server)
/// - `Tgi` — value `tgi` (HuggingFace text-generation-inference)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendType {
    #[default]
    Ollama,
    OpenAi,
    OpenLlm,
    LlamaCpp,
    Tgi,
}

impl BackendType {
    /// Parse from a string (case-insensitive). Accepts aliases.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ollama" => Ok(Self::Ollama),
            // OpenAI-compatible family
            "openai" | "openai-compatible" | "openaicompat" | "openai_compat" => Ok(Self::OpenAi),
            "lmstudio" | "lm-studio" | "lm_studio" => Ok(Self::OpenAi),
            "vllm" => Ok(Self::OpenAi),
            "localai" | "local-ai" | "local_ai" => Ok(Self::OpenAi),
            "groq" | "openrouter" | "together" | "fireworks" | "anyscale" => Ok(Self::OpenAi),
            // OpenLLM
            "openllm" | "open-llm" | "open_llm" | "bentoml" => Ok(Self::OpenLlm),
            // llama.cpp
            "llamacpp" | "llama-cpp" | "llama_cpp" | "llama" => Ok(Self::LlamaCpp),
            // TGI
            "tgi" | "text-generation-inference" => Ok(Self::Tgi),
            other => Err(anyhow!(
                "unknown backend type '{}' (expected: ollama, openai [lmstudio/vllm/localai/groq/openrouter/...], openllm, llamacpp, tgi)",
                other
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenAi => "openai",
            Self::OpenLlm => "openllm",
            Self::LlamaCpp => "llamacpp",
            Self::Tgi => "tgi",
        }
    }
}

impl BackendType {
    /// Default model identifiers shipped by each backend vendor.
    ///
    /// These are the canonical names used by each engine's API:
    /// - Ollama: tag-based model names pulled from the Ollama library
    ///   (e.g. `gemma3:4b`).
    /// - OpenAI: model ids used by the OpenAI Chat Completions API
    ///   (and the OpenAI-compatible adapters: Groq, OpenRouter, Together,
    ///   LM Studio, vLLM, LocalAI, llama.cpp OpenAI shim, TGI OpenAI shim).
    /// - Llama.cpp: HuggingFace repo names commonly served via llama.cpp's
    ///   GGUF loader.
    /// - TGI: HuggingFace repo names typically deployed with TGI.
    pub fn default_models(&self) -> &'static [&'static str] {
        match self {
            Self::Ollama => &[
                "gemma3:4b",       // Google Gemma 3 4B (latest small)
                "llama3.3:latest", // Meta Llama 3.3 70B (default tag)
                "deepseek-r1:8b",  // DeepSeek R1 distilled 8B
                "qwen2.5:7b",      // Alibaba Qwen 2.5 7B
            ],
            Self::OpenAi => &[
                "gpt-4o-mini",   // OpenAI GPT-4o mini (cheap default)
                "gpt-4o",        // OpenAI GPT-4o
                "gpt-4-turbo",   // OpenAI GPT-4 Turbo
                "gpt-3.5-turbo", // OpenAI GPT-3.5 Turbo
            ],
            Self::OpenLlm => &[
                "meta-llama/Llama-3.1-8B-Instruct",   // Meta Llama 3.1 8B Instruct
                "mistralai/Mistral-7B-Instruct-v0.3", // Mistral 7B Instruct v0.3
                "Qwen/Qwen2.5-7B-Instruct",           // Alibaba Qwen 2.5 7B Instruct
                "google/gemma-2-9b-it",               // Google Gemma 2 9B IT
            ],
            Self::LlamaCpp => &[
                "Qwen/Qwen2.5-7B-Instruct",           // Alibaba Qwen 2.5 7B Instruct
                "meta-llama/Llama-3.1-8B-Instruct",   // Meta Llama 3.1 8B Instruct
                "mistralai/Mistral-7B-Instruct-v0.3", // Mistral 7B Instruct v0.3
                "google/gemma-2-9b-it",               // Google Gemma 2 9B IT (gguf form)
            ],
            Self::Tgi => &[
                "meta-llama/Llama-3.1-8B-Instruct",   // Meta Llama 3.1 8B Instruct
                "mistralai/Mistral-7B-Instruct-v0.3", // Mistral 7B Instruct v0.3
                "google/gemma-2-9b-it",               // Google Gemma 2 9B IT
                "Qwen/Qwen2.5-7B-Instruct",           // Alibaba Qwen 2.5 7B Instruct
            ],
        }
    }

    /// Human-readable description of the backend (used in startup logs).
    pub fn description(&self) -> &'static str {
        match self {
            Self::Ollama => "Ollama (local, /api/generate + /api/tags)",
            Self::OpenAi => "OpenAI-compatible (LM Studio, vLLM, OpenAI, Groq, OpenRouter, ...)",
            Self::OpenLlm => "BentoML OpenLLM (/v1/generate + /v1/chat/completions + /v1/models)",
            Self::LlamaCpp => "llama.cpp server (native /completion + /props + /health)",
            Self::Tgi => "HuggingFace text-generation-inference (/generate + /info)",
        }
    }
}

/// Shared mock inference fallback used by every real backend when the local
/// engine is unavailable. Keeps the worker operational for devnet/testing even
/// when no inference server is running.
pub(crate) fn mock_generate(
    model: &str,
    prompt: &str,
    max_tokens: u64,
) -> Result<GenerationResponse> {
    tracing::info!("Using mock inference for model: {}", model);

    let mock_text = format!(
        "Mock inference response for prompt: '{}'. This is a simulated AI response \
        generated because the configured inference backend is not available. In \
        production, this would be replaced with actual model output.",
        prompt.chars().take(50).collect::<String>()
    );

    let tokens: Vec<String> = mock_text
        .split_whitespace()
        .take(max_tokens as usize)
        .map(|s| s.to_string())
        .collect();
    let token_count = tokens.len() as u64;

    Ok(GenerationResponse {
        text: tokens.join(" "),
        tokens,
        token_count,
        model: model.to_string(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_backend_type_parse() {
        assert_eq!(BackendType::parse("ollama").unwrap(), BackendType::Ollama);
        assert_eq!(BackendType::parse("OLLAMA").unwrap(), BackendType::Ollama);
        assert_eq!(BackendType::parse("openai").unwrap(), BackendType::OpenAi);
        assert_eq!(
            BackendType::parse("openai-compatible").unwrap(),
            BackendType::OpenAi
        );
        assert_eq!(BackendType::parse("openllm").unwrap(), BackendType::OpenLlm);
        assert_eq!(BackendType::parse("bentoml").unwrap(), BackendType::OpenLlm);
        assert_eq!(
            BackendType::parse("llamacpp").unwrap(),
            BackendType::LlamaCpp
        );
        assert_eq!(
            BackendType::parse("llama-cpp").unwrap(),
            BackendType::LlamaCpp
        );
        assert_eq!(BackendType::parse("tgi").unwrap(), BackendType::Tgi);
        assert!(BackendType::parse("unknown").is_err());
        assert_eq!(BackendType::default(), BackendType::Ollama);
        assert_eq!(BackendType::Ollama.as_str(), "ollama");
        assert_eq!(BackendType::OpenAi.as_str(), "openai");
        assert_eq!(BackendType::OpenLlm.as_str(), "openllm");
        assert_eq!(BackendType::LlamaCpp.as_str(), "llamacpp");
        assert_eq!(BackendType::Tgi.as_str(), "tgi");
    }

    #[test]
    fn test_backend_type_default_models() {
        // Ollama: tag-based names
        let ollama = BackendType::Ollama.default_models();
        assert!(!ollama.is_empty());
        assert!(
            ollama.contains(&"gemma3:4b"),
            "Ollama should ship with gemma3:4b"
        );
        assert!(ollama.contains(&"llama3.3:latest"));
        assert!(ollama.contains(&"deepseek-r1:8b"));

        // OpenAI: gpt-* ids
        let openai = BackendType::OpenAi.default_models();
        assert!(!openai.is_empty());
        assert!(
            openai.contains(&"gpt-4o-mini"),
            "OpenAI should ship with gpt-4o-mini"
        );
        assert!(openai.contains(&"gpt-4o"));
        assert!(openai.contains(&"gpt-3.5-turbo"));

        // OpenLLM: HF repo names (BentoML OpenLLM)
        let openllm = BackendType::OpenLlm.default_models();
        assert!(!openllm.is_empty());
        assert!(openllm.contains(&"meta-llama/Llama-3.1-8B-Instruct"));
        assert!(openllm.contains(&"mistralai/Mistral-7B-Instruct-v0.3"));
        assert!(openllm.contains(&"Qwen/Qwen2.5-7B-Instruct"));

        // Llama.cpp: HF repo names
        let llamacpp = BackendType::LlamaCpp.default_models();
        assert!(!llamacpp.is_empty());
        assert!(llamacpp.contains(&"Qwen/Qwen2.5-7B-Instruct"));
        assert!(llamacpp.contains(&"meta-llama/Llama-3.1-8B-Instruct"));
        assert!(llamacpp.contains(&"mistralai/Mistral-7B-Instruct-v0.3"));

        // TGI: HF repo names
        let tgi = BackendType::Tgi.default_models();
        assert!(!tgi.is_empty());
        assert!(tgi.contains(&"meta-llama/Llama-3.1-8B-Instruct"));
        assert!(tgi.contains(&"mistralai/Mistral-7B-Instruct-v0.3"));

        // Each backend ships at least 3 distinct defaults
        for backend in [
            BackendType::Ollama,
            BackendType::OpenAi,
            BackendType::OpenLlm,
            BackendType::LlamaCpp,
            BackendType::Tgi,
        ] {
            assert!(
                backend.default_models().len() >= 3,
                "{:?} should ship at least 3 default models",
                backend
            );
        }
    }

    #[test]
    fn test_backend_type_description() {
        for backend in [
            BackendType::Ollama,
            BackendType::OpenAi,
            BackendType::OpenLlm,
            BackendType::LlamaCpp,
            BackendType::Tgi,
        ] {
            let desc = backend.description();
            assert!(!desc.is_empty());
        }
    }

    #[test]
    fn test_mock_generate_shared() {
        let r = mock_generate("test-model", "hello world", 10).unwrap();
        assert_eq!(r.model, "test-model");
        assert!(r.token_count > 0);
        assert!(r.token_count <= 10);
    }
}
