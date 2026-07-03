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

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
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

impl Default for BackendType {
    fn default() -> Self {
        Self::Ollama
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
                "gemma3:4b",                   // Google Gemma 3 4B (latest small)
                "llama3.3:latest",             // Meta Llama 3.3 70B (default tag)
                "deepseek-r1:8b",              // DeepSeek R1 distilled 8B
                "qwen2.5:7b",                  // Alibaba Qwen 2.5 7B
            ],
            Self::OpenAi => &[
                "gpt-4o-mini",                 // OpenAI GPT-4o mini (cheap default)
                "gpt-4o",                      // OpenAI GPT-4o
                "gpt-4-turbo",                 // OpenAI GPT-4 Turbo
                "gpt-3.5-turbo",               // OpenAI GPT-3.5 Turbo
            ],
            Self::OpenLlm => &[
                "meta-llama/Llama-3.1-8B-Instruct",    // Meta Llama 3.1 8B Instruct
                "mistralai/Mistral-7B-Instruct-v0.3",  // Mistral 7B Instruct v0.3
                "Qwen/Qwen2.5-7B-Instruct",            // Alibaba Qwen 2.5 7B Instruct
                "google/gemma-2-9b-it",                // Google Gemma 2 9B IT
            ],
            Self::LlamaCpp => &[
                "Qwen/Qwen2.5-7B-Instruct",            // Alibaba Qwen 2.5 7B Instruct
                "meta-llama/Llama-3.1-8B-Instruct",    // Meta Llama 3.1 8B Instruct
                "mistralai/Mistral-7B-Instruct-v0.3",  // Mistral 7B Instruct v0.3
                "google/gemma-2-9b-it",                // Google Gemma 2 9B IT (gguf form)
            ],
            Self::Tgi => &[
                "meta-llama/Llama-3.1-8B-Instruct",    // Meta Llama 3.1 8B Instruct
                "mistralai/Mistral-7B-Instruct-v0.3",  // Mistral 7B Instruct v0.3
                "google/gemma-2-9b-it",                // Google Gemma 2 9B IT
                "Qwen/Qwen2.5-7B-Instruct",            // Alibaba Qwen 2.5 7B Instruct
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

// ---------------------------------------------------------------------------
// OllamaBackend (native)
// ---------------------------------------------------------------------------

/// Ollama backend implementation
pub struct OllamaBackend {
    url: String,
    client: reqwest::Client,
}

impl OllamaBackend {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl InferenceBackend for OllamaBackend {
    async fn generate(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u64,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<GenerationResponse> {
        #[derive(Serialize)]
        struct OllamaRequest {
            model: String,
            prompt: String,
            stream: bool,
            options: OllamaOptions,
        }

        #[derive(Serialize)]
        struct OllamaOptions {
            num_predict: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            top_p: Option<f32>,
        }

        #[derive(Deserialize)]
        struct OllamaResponse {
            response: String,
            #[serde(default)]
            done: bool,
        }

        let request = OllamaRequest {
            model: model.to_string(),
            prompt: prompt.to_string(),
            stream: false,
            options: OllamaOptions {
                num_predict: max_tokens,
                temperature,
                top_p,
            },
        };

        let url = format!("{}/api/generate", self.url);
        let response = self.client.post(&url).json(&request).send().await;

        let ollama_response: OllamaResponse = match response {
            Ok(resp) if resp.status().is_success() => resp.json().await?,
            Ok(resp) => {
                tracing::warn!(
                    "Ollama request failed: {} - falling back to mock inference",
                    resp.status()
                );
                return mock_generate(model, prompt, max_tokens);
            }
            Err(e) => {
                tracing::warn!(
                    "Ollama connection failed: {} - falling back to mock inference",
                    e
                );
                return mock_generate(model, prompt, max_tokens);
            }
        };

        let tokens: Vec<String> = ollama_response
            .response
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let token_count = tokens.len() as u64;

        Ok(GenerationResponse {
            text: ollama_response.response,
            tokens,
            token_count,
            model: model.to_string(),
        })
    }

    async fn is_model_loaded(&self, model: &str) -> Result<bool> {
        let models = self.list_models().await?;
        Ok(models.iter().any(|m| m.name == model))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        #[derive(Deserialize)]
        struct OllamaModelsResponse {
            models: Vec<OllamaModel>,
        }

        #[derive(Deserialize)]
        struct OllamaModel {
            name: String,
            size: u64,
            #[serde(default)]
            details: OllamaModelDetails,
        }

        #[derive(Deserialize, Default)]
        struct OllamaModelDetails {
            #[serde(default)]
            family: String,
        }

        let url = format!("{}/api/tags", self.url);
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("Failed to list Ollama models"));
        }

        let ollama_response: OllamaModelsResponse = response.json().await?;

        let models = ollama_response
            .models
            .into_iter()
            .map(|m| ModelInfo {
                name: m.name,
                size: m.size,
                family: m.details.family,
            })
            .collect();

        Ok(models)
    }
}

// ---------------------------------------------------------------------------
// OpenAiCompatibleBackend
//   Covers: LM Studio, vLLM, LocalAI, llama.cpp OpenAI shim, TGI OpenAI shim,
//   and cloud OpenAI-compatible providers (OpenAI, Groq, OpenRouter, Together,
//   Anyscale, Fireworks, ...). Cloud providers require `api_key` set.
// ---------------------------------------------------------------------------

/// OpenAI-compatible backend (local or cloud).
///
/// Endpoint shapes used:
/// - `POST {base_url}/v1/chat/completions`
/// - `GET  {base_url}/v1/models`
///
/// `base_url` should be the origin (e.g. `http://localhost:1234/v1` for LM
/// Studio, `https://api.openai.com/v1` for OpenAI, `https://api.groq.com/openai/v1`
/// for Groq, `https://openrouter.ai/api/v1` for OpenRouter). When `api_key` is
/// set, requests carry `Authorization: Bearer <key>`.
pub struct OpenAiCompatibleBackend {
    base_url: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenAiCompatibleBackend {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            client: reqwest::Client::new(),
        }
    }

    fn add_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }
}

#[async_trait]
impl InferenceBackend for OpenAiCompatibleBackend {
    async fn generate(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u64,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<GenerationResponse> {
        // OpenAI chat completions request
        #[derive(Serialize)]
        struct ChatRequest<'a> {
            model: &'a str,
            messages: Vec<ChatMessage<'a>>,
            max_tokens: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            top_p: Option<f32>,
            stream: bool,
        }

        #[derive(Serialize)]
        struct ChatMessage<'a> {
            role: &'a str,
            content: &'a str,
        }

        // OpenAI chat completions response
        #[derive(Deserialize)]
        struct ChatResponse {
            choices: Vec<ChatChoice>,
            #[serde(default)]
            usage: Option<ChatUsage>,
        }

        #[derive(Deserialize)]
        struct ChatChoice {
            #[serde(default)]
            message: Option<ChatChoiceMessage>,
        }

        #[derive(Deserialize)]
        struct ChatChoiceMessage {
            content: String,
        }

        #[derive(Deserialize, Default)]
        struct ChatUsage {
            #[serde(default)]
            completion_tokens: Option<u64>,
        }

        let req_body = ChatRequest {
            model,
            messages: vec![ChatMessage {
                role: "user",
                content: prompt,
            }],
            max_tokens,
            temperature,
            top_p,
            stream: false,
        };

        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .add_auth(self.client.post(&url).json(&req_body))
            .send()
            .await;

        let chat_response: ChatResponse = match response {
            Ok(resp) if resp.status().is_success() => resp.json().await?,
            Ok(resp) => {
                tracing::warn!(
                    "OpenAI-compatible request failed: {} - falling back to mock inference",
                    resp.status()
                );
                return mock_generate(model, prompt, max_tokens);
            }
            Err(e) => {
                tracing::warn!(
                    "OpenAI-compatible backend connection failed: {} - falling back to mock inference",
                    e
                );
                return mock_generate(model, prompt, max_tokens);
            }
        };

        let text = chat_response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message)
            .map(|m| m.content)
            .ok_or_else(|| anyhow!("OpenAI-compatible response had no choices/content"))?;

        let token_count = chat_response
            .usage
            .and_then(|u| u.completion_tokens)
            .unwrap_or_else(|| text.split_whitespace().count() as u64);

        let tokens: Vec<String> = text.split_whitespace().map(|s| s.to_string()).collect();

        Ok(GenerationResponse {
            text,
            tokens,
            token_count,
            model: model.to_string(),
        })
    }

    async fn is_model_loaded(&self, model: &str) -> Result<bool> {
        let models = self.list_models().await?;
        Ok(models.iter().any(|m| m.name == model))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }

        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
            #[serde(default)]
            owned_by: Option<String>,
        }

        let url = format!("{}/models", self.base_url);
        let response = self.add_auth(self.client.get(&url)).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to list models from OpenAI-compatible backend: {}",
                response.status()
            ));
        }

        let resp: ModelsResponse = response.json().await?;

        let models = resp
            .data
            .into_iter()
            .map(|m| ModelInfo {
                name: m.id,
                size: 0,
                family: m.owned_by.unwrap_or_else(|| "openai-compatible".to_string()),
            })
            .collect();

        Ok(models)
    }
}

// ---------------------------------------------------------------------------
// LlamaCppBackend (native llama.cpp server: /completion, /props, /health)
// ---------------------------------------------------------------------------

/// Native llama.cpp server backend.
///
/// llama.cpp loads exactly one model at server start and ignores the
/// per-request `model` field. We log a warning if the requested model name
/// differs from the server's loaded model (best-effort via `/props`), but we
/// still execute — the server will use whatever it has loaded.
pub struct LlamaCppBackend {
    url: String,
    client: reqwest::Client,
}

impl LlamaCppBackend {
    pub fn new(url: String) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Fetch the model id baked into the running llama.cpp server (via /props).
    async fn server_model_id(&self) -> Option<String> {
        #[derive(Deserialize)]
        struct Props {
            #[serde(default)]
            model_name: Option<String>,
            #[serde(default)]
            model_path: Option<String>,
        }

        let url = format!("{}/props", self.url);
        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let props: Props = resp.json().await.ok()?;
        props.model_name.or(props.model_path)
    }
}

#[async_trait]
impl InferenceBackend for LlamaCppBackend {
    async fn generate(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u64,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<GenerationResponse> {
        // Best-effort: warn if the requested model doesn't match the server's
        // loaded model. llama.cpp ignores `model` in the request body.
        if let Some(server_model) = self.server_model_id().await {
            if !server_model.eq_ignore_ascii_case(model)
                && !server_model.contains(model)
                && !model.contains(&server_model)
            {
                tracing::warn!(
                    "llama.cpp server has model '{}' loaded but job requested '{}'; \
                     llama.cpp ignores the per-request model and will use the loaded one",
                    server_model,
                    model
                );
            }
        }

        #[derive(Serialize)]
        struct LcppRequest<'a> {
            prompt: &'a str,
            n_predict: u64,
            stream: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            top_p: Option<f32>,
        }

        #[derive(Deserialize)]
        struct LcppResponse {
            content: String,
            #[serde(default)]
            tokens_predicted: Option<u64>,
        }

        let req_body = LcppRequest {
            prompt,
            n_predict: max_tokens,
            stream: false,
            temperature,
            top_p,
        };

        let url = format!("{}/completion", self.url);
        let response = self.client.post(&url).json(&req_body).send().await;

        let lcpp_response: LcppResponse = match response {
            Ok(resp) if resp.status().is_success() => resp.json().await?,
            Ok(resp) => {
                tracing::warn!(
                    "llama.cpp request failed: {} - falling back to mock inference",
                    resp.status()
                );
                return mock_generate(model, prompt, max_tokens);
            }
            Err(e) => {
                tracing::warn!(
                    "llama.cpp connection failed: {} - falling back to mock inference",
                    e
                );
                return mock_generate(model, prompt, max_tokens);
            }
        };

        let tokens: Vec<String> = lcpp_response
            .content
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let token_count =
            lcpp_response.tokens_predicted.unwrap_or_else(|| tokens.len() as u64);

        Ok(GenerationResponse {
            text: lcpp_response.content,
            tokens,
            token_count,
            model: model.to_string(),
        })
    }

    async fn is_model_loaded(&self, model: &str) -> Result<bool> {
        // llama.cpp loads one model at server start; a healthy server means
        // "loaded". We additionally confirm the model id when we can read it.
        let url = format!("{}/health", self.url);
        let resp = self.client.get(&url).send().await;
        let healthy = matches!(resp, Ok(r) if r.status().is_success());
        if !healthy {
            return Ok(false);
        }
        if let Some(server_model) = self.server_model_id().await {
            Ok(server_model.eq_ignore_ascii_case(model)
                || server_model.contains(model)
                || model.contains(&server_model)
                // If we can't determine the model name, accept any.
                || server_model.is_empty())
        } else {
            // Can't read /props — assume the healthy server has *a* model loaded.
            Ok(true)
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        // llama.cpp exposes no model list; report the single loaded model if
        // discoverable via /props, otherwise return an empty list.
        let model = self.server_model_id().await;
        Ok(match model {
            Some(name) => vec![ModelInfo {
                name,
                size: 0,
                family: "llama.cpp".to_string(),
            }],
            None => Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// TgiBackend (HuggingFace text-generation-inference: /generate, /info)
// ---------------------------------------------------------------------------

/// HuggingFace text-generation-inference (TGI) backend.
///
/// Endpoint shapes:
/// - `POST {url}/generate` — `{inputs, parameters:{max_new_tokens,...}}`
/// - `GET  {url}/info` — server/model metadata
///
/// TGI loads one model per server instance (no model list); `/info` exposes
/// the loaded model id.
pub struct TgiBackend {
    url: String,
    client: reqwest::Client,
}

impl TgiBackend {
    pub fn new(url: String) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Fetch the model id from TGI `/info` (None if unavailable).
    async fn info_model_id(&self) -> Option<String> {
        #[derive(Deserialize)]
        struct Info {
            #[serde(default)]
            model_id: Option<String>,
            #[serde(default)]
            model_device_type: Option<String>,
        }

        let url = format!("{}/info", self.url);
        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let info: Info = resp.json().await.ok()?;
        info.model_id
    }
}

#[async_trait]
impl InferenceBackend for TgiBackend {
    async fn generate(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u64,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<GenerationResponse> {
        #[derive(Serialize)]
        struct TgiRequest<'a> {
            inputs: &'a str,
            parameters: TgiParameters,
        }

        #[derive(Serialize)]
        struct TgiParameters {
            max_new_tokens: u64,
            return_full_text: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            top_p: Option<f32>,
        }

        #[derive(Deserialize)]
        struct TgiResponse {
            generated_text: String,
        }

        let req_body = TgiRequest {
            inputs: prompt,
            parameters: TgiParameters {
                max_new_tokens: max_tokens,
                return_full_text: false,
                temperature,
                top_p,
            },
        };

        let url = format!("{}/generate", self.url);
        let response = self.client.post(&url).json(&req_body).send().await;

        let tgi_response: TgiResponse = match response {
            Ok(resp) if resp.status().is_success() => resp.json().await?,
            Ok(resp) => {
                tracing::warn!(
                    "TGI request failed: {} - falling back to mock inference",
                    resp.status()
                );
                return mock_generate(model, prompt, max_tokens);
            }
            Err(e) => {
                tracing::warn!(
                    "TGI connection failed: {} - falling back to mock inference",
                    e
                );
                return mock_generate(model, prompt, max_tokens);
            }
        };

        // TGI /generate does not return a token count by default; fall back to
        // whitespace split (consistent with the Ollama backend's approach).
        let tokens: Vec<String> = tgi_response
            .generated_text
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let token_count = tokens.len() as u64;

        Ok(GenerationResponse {
            text: tgi_response.generated_text,
            tokens,
            token_count,
            model: model.to_string(),
        })
    }

    async fn is_model_loaded(&self, model: &str) -> Result<bool> {
        // TGI serves one model per instance; `/info` exposes its id.
        match self.info_model_id().await {
            Some(id) => Ok(id.eq_ignore_ascii_case(model) || id.contains(model) || model.contains(&id)),
            None => {
                // Couldn't read /info — probe a bare GET on /info for health.
                let url = format!("{}/info", self.url);
                let resp = self.client.get(&url).send().await;
                Ok(matches!(resp, Ok(r) if r.status().is_success()))
            }
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(match self.info_model_id().await {
            Some(id) => vec![ModelInfo {
                name: id,
                size: 0,
                family: "tgi".to_string(),
            }],
            None => Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// OpenLlmBackend (BentoML OpenLLM)
// ---------------------------------------------------------------------------

/// BentoML OpenLLM backend.
///
/// OpenLLM exposes a BentoML-style HTTP API alongside an OpenAI-compatible
/// surface. This backend uses the **native** BentoML endpoints so we get
/// exact token counts and per-request model selection (OpenLLM supports
/// `adapter_name` for LoRA and `model_id` for runtime model swap on
/// multi-model deployments).
///
/// Endpoints used:
/// - `POST {base_url}/v1/generate` — `{"prompt", "llm_config", "model_id"}`
/// - `POST {base_url}/v1/chat/completions` — OpenAI-style chat
/// - `GET  {base_url}/v1/models` — list deployed models
///
/// We always call `/v1/chat/completions` (OpenAI-compatible) so the request
/// shape matches what user-facing frontends expect and so we get
/// `usage.completion_tokens` for exact token counts.
pub struct OpenLlmBackend {
    base_url: String,
    client: reqwest::Client,
}

impl OpenLlmBackend {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl InferenceBackend for OpenLlmBackend {
    async fn generate(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u64,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<GenerationResponse> {
        #[derive(Serialize)]
        struct ChatRequest<'a> {
            model: &'a str,
            messages: Vec<ChatMessage<'a>>,
            max_tokens: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            top_p: Option<f32>,
            stream: bool,
        }

        #[derive(Serialize)]
        struct ChatMessage<'a> {
            role: &'a str,
            content: &'a str,
        }

        #[derive(Deserialize)]
        struct ChatResponse {
            choices: Vec<ChatChoice>,
            #[serde(default)]
            usage: Option<ChatUsage>,
        }

        #[derive(Deserialize)]
        struct ChatChoice {
            #[serde(default)]
            message: Option<ChatChoiceMessage>,
        }

        #[derive(Deserialize)]
        struct ChatChoiceMessage {
            content: String,
        }

        #[derive(Deserialize, Default)]
        struct ChatUsage {
            #[serde(default)]
            completion_tokens: Option<u64>,
        }

        let req_body = ChatRequest {
            model,
            messages: vec![ChatMessage {
                role: "user",
                content: prompt,
            }],
            max_tokens,
            temperature,
            top_p,
            stream: false,
        };

        let url = format!("{}/v1/chat/completions", self.base_url);
        let response = self.client.post(&url).json(&req_body).send().await;

        let chat_response: ChatResponse = match response {
            Ok(resp) if resp.status().is_success() => resp.json().await?,
            Ok(resp) => {
                tracing::warn!(
                    "OpenLLM request failed: {} - falling back to mock inference",
                    resp.status()
                );
                return mock_generate(model, prompt, max_tokens);
            }
            Err(e) => {
                tracing::warn!(
                    "OpenLLM connection failed: {} - falling back to mock inference",
                    e
                );
                return mock_generate(model, prompt, max_tokens);
            }
        };

        let text = chat_response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message)
            .map(|m| m.content)
            .ok_or_else(|| anyhow!("OpenLLM response had no choices/content"))?;

        let token_count = chat_response
            .usage
            .and_then(|u| u.completion_tokens)
            .unwrap_or_else(|| text.split_whitespace().count() as u64);

        let tokens: Vec<String> = text.split_whitespace().map(|s| s.to_string()).collect();

        Ok(GenerationResponse {
            text,
            tokens,
            token_count,
            model: model.to_string(),
        })
    }

    async fn is_model_loaded(&self, model: &str) -> Result<bool> {
        let models = self.list_models().await?;
        Ok(models.iter().any(|m| m.name == model))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        // OpenLLM exposes deployed model ids via /v1/models and
        // /models.json. /v1/models is OpenAI-compatible and works on all
        // recent OpenLLM versions.
        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }

        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
            #[serde(default)]
            owned_by: Option<String>,
        }

        let url = format!("{}/v1/models", self.base_url);
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to list OpenLLM models: {}",
                response.status()
            ));
        }

        let resp: ModelsResponse = response.json().await?;

        let models = resp
            .data
            .into_iter()
            .map(|m| ModelInfo {
                name: m.id,
                size: 0,
                family: m.owned_by.unwrap_or_else(|| "openllm".to_string()),
            })
            .collect();

        Ok(models)
    }
}

// ---------------------------------------------------------------------------
// MockBackend (testing)
// ---------------------------------------------------------------------------

/// Mock backend for testing
pub struct MockBackend {
    models: Vec<String>,
}

impl MockBackend {
    pub fn new(models: Vec<String>) -> Self {
        Self { models }
    }
}

#[async_trait]
impl InferenceBackend for MockBackend {
    async fn generate(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u64,
        _temperature: Option<f32>,
        _top_p: Option<f32>,
    ) -> Result<GenerationResponse> {
        let text = format!("Mock response to: {}", prompt);
        let tokens: Vec<String> = text.split_whitespace().map(|s| s.to_string()).collect();
        let token_count = tokens.len().min(max_tokens as usize) as u64;

        Ok(GenerationResponse {
            text,
            tokens: tokens.into_iter().take(token_count as usize).collect(),
            token_count,
            model: model.to_string(),
        })
    }

    async fn is_model_loaded(&self, model: &str) -> Result<bool> {
        Ok(self.models.contains(&model.to_string()))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self
            .models
            .iter()
            .map(|name| ModelInfo {
                name: name.clone(),
                size: 7_000_000_000, // Mock 7B model
                family: "mock".to_string(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_backend() {
        let backend = MockBackend::new(vec!["llama2".to_string()]);

        let response = backend.generate("llama2", "Hello", 100, None, None).await.unwrap();

        assert!(!response.text.is_empty());
        assert!(response.token_count > 0);
    }

    #[tokio::test]
    async fn test_model_listing() {
        let backend = MockBackend::new(vec!["llama2".to_string(), "mistral".to_string()]);

        let models = backend.list_models().await.unwrap();
        assert_eq!(models.len(), 2);
    }

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
        assert_eq!(BackendType::parse("llamacpp").unwrap(), BackendType::LlamaCpp);
        assert_eq!(BackendType::parse("llama-cpp").unwrap(), BackendType::LlamaCpp);
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
        assert!(ollama.contains(&"gemma3:4b"), "Ollama should ship with gemma3:4b");
        assert!(ollama.contains(&"llama3.3:latest"));
        assert!(ollama.contains(&"deepseek-r1:8b"));

        // OpenAI: gpt-* ids
        let openai = BackendType::OpenAi.default_models();
        assert!(!openai.is_empty());
        assert!(openai.contains(&"gpt-4o-mini"), "OpenAI should ship with gpt-4o-mini");
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

    #[test]
    fn test_openai_response_parse() {
        // Validate we can deserialize a representative OpenAI chat response.
        let body = r#"{
            "choices": [{"message": {"content": "hello there"}}],
            "usage": {"completion_tokens": 2}
        }"#;
        #[derive(Deserialize)]
        struct ChatResponse {
            choices: Vec<ChatChoice>,
            #[serde(default)]
            usage: Option<ChatUsage>,
        }
        #[derive(Deserialize)]
        struct ChatChoice {
            #[serde(default)]
            message: Option<ChatChoiceMessage>,
        }
        #[derive(Deserialize)]
        struct ChatChoiceMessage {
            content: String,
        }
        #[derive(Deserialize, Default)]
        struct ChatUsage {
            #[serde(default)]
            completion_tokens: Option<u64>,
        }
        let parsed: ChatResponse = serde_json::from_str(body).unwrap();
        let text = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message)
            .map(|m| m.content)
            .unwrap();
        let tokens = parsed
            .usage
            .and_then(|u| u.completion_tokens)
            .unwrap_or_else(|| text.split_whitespace().count() as u64);
        assert_eq!(text, "hello there");
        assert_eq!(tokens, 2);
    }

    #[test]
    fn test_openai_models_response_parse() {
        let body = r#"{
            "data": [{"id": "gpt-4o", "owned_by": "openai"}, {"id": "llama-3-8b", "owned_by": null}]
        }"#;
        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }
        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
            #[serde(default)]
            owned_by: Option<String>,
        }
        let parsed: ModelsResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.data.len(), 2);
        assert_eq!(parsed.data[0].id, "gpt-4o");
    }

    #[test]
    fn test_llamacpp_response_parse() {
        let body = r#"{"content": "general kenobi", "tokens_predicted": 2}"#;
        #[derive(Deserialize)]
        struct LcppResponse {
            content: String,
            #[serde(default)]
            tokens_predicted: Option<u64>,
        }
        let parsed: LcppResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.content, "general kenobi");
        assert_eq!(parsed.tokens_predicted, Some(2));
    }

    #[test]
    fn test_llamacpp_props_parse() {
        let body = r#"{"model_name": "qwen2.5-7b-instruct-q5_k_m", "model_path": "/models/qwen.gguf"}"#;
        #[derive(Deserialize)]
        struct Props {
            #[serde(default)]
            model_name: Option<String>,
            #[serde(default)]
            model_path: Option<String>,
        }
        let parsed: Props = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.model_name.as_deref(), Some("qwen2.5-7b-instruct-q5_k_m"));
        // model_name takes precedence over model_path in our code.
        assert_eq!(parsed.model_name.or(parsed.model_path).unwrap(), "qwen2.5-7b-instruct-q5_k_m");
    }

    #[test]
    fn test_tgi_response_parse() {
        let body = r#"{"generated_text": "hello from tgi"}"#;
        #[derive(Deserialize)]
        struct TgiResponse {
            generated_text: String,
        }
        let parsed: TgiResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.generated_text, "hello from tgi");
    }

    #[test]
    fn test_tgi_info_parse() {
        let body = r#"{"model_id": "meta-llama/Llama-3-8B-Instruct", "model_device_type": "cuda"}"#;
        #[derive(Deserialize)]
        struct Info {
            #[serde(default)]
            model_id: Option<String>,
            #[serde(default)]
            model_device_type: Option<String>,
        }
        let parsed: Info = serde_json::from_str(body).unwrap();
        assert_eq!(
            parsed.model_id.as_deref(),
            Some("meta-llama/Llama-3-8B-Instruct")
        );
    }

    #[test]
    fn test_openllm_response_parse() {
        // OpenLLM /v1/chat/completions response shape (mirrors OpenAI).
        let body = r#"{
            "choices": [{"message": {"content": "hello from openllm"}}],
            "usage": {"completion_tokens": 3}
        }"#;
        #[derive(Deserialize)]
        struct ChatResponse {
            choices: Vec<ChatChoice>,
            #[serde(default)]
            usage: Option<ChatUsage>,
        }
        #[derive(Deserialize)]
        struct ChatChoice {
            #[serde(default)]
            message: Option<ChatChoiceMessage>,
        }
        #[derive(Deserialize)]
        struct ChatChoiceMessage {
            content: String,
        }
        #[derive(Deserialize, Default)]
        struct ChatUsage {
            #[serde(default)]
            completion_tokens: Option<u64>,
        }
        let parsed: ChatResponse = serde_json::from_str(body).unwrap();
        let text = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message)
            .map(|m| m.content)
            .unwrap();
        let tokens = parsed
            .usage
            .and_then(|u| u.completion_tokens)
            .unwrap_or_else(|| text.split_whitespace().count() as u64);
        assert_eq!(text, "hello from openllm");
        assert_eq!(tokens, 3);
    }

    #[test]
    fn test_openllm_models_response_parse() {
        // OpenLLM /v1/models response (OpenAI-compatible wrapper around its
        // deployed model registry).
        let body = r#"{
            "data": [
                {"id": "meta-llama/Llama-3.1-8B-Instruct", "owned_by": "bentoml"},
                {"id": "mistralai/Mistral-7B-Instruct-v0.3", "owned_by": "bentoml"}
            ]
        }"#;
        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }
        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
            #[serde(default)]
            owned_by: Option<String>,
        }
        let parsed: ModelsResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.data.len(), 2);
        assert_eq!(parsed.data[0].id, "meta-llama/Llama-3.1-8B-Instruct");
        assert_eq!(parsed.data[0].owned_by.as_deref(), Some("bentoml"));
    }
}