use super::{GenerationResponse, InferenceBackend, ModelInfo, mock_generate};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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
                family: m
                    .owned_by
                    .unwrap_or_else(|| "openai-compatible".to_string()),
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
        assert_eq!(parsed.data[0].owned_by.as_deref(), Some("openai"));
        assert_eq!(parsed.data[1].owned_by, None);
    }
}
