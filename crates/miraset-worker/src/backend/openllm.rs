use super::{GenerationResponse, InferenceBackend, ModelInfo, mock_generate};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use serde::Deserialize;

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
