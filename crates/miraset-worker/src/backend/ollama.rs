use super::{GenerationResponse, InferenceBackend, ModelInfo, mock_generate};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Ollama backend implementation
pub struct OllamaBackend {
    url: String,
    client: reqwest::Client,
    allow_mock_fallback: bool,
}

impl OllamaBackend {
    #[must_use]
    pub fn new(url: String) -> Self {
        Self::with_mock_fallback(url, true)
    }

    #[must_use]
    pub fn with_mock_fallback(url: String, allow_mock_fallback: bool) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
            allow_mock_fallback,
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
                if !self.allow_mock_fallback {
                    return Err(anyhow!(
                        "Ollama returned unsuccessful status {}",
                        resp.status()
                    ));
                }
                tracing::warn!(
                    "Ollama request failed: {} - falling back to mock inference",
                    resp.status()
                );
                return mock_generate(model, prompt, max_tokens);
            }
            Err(e) => {
                if !self.allow_mock_fallback {
                    return Err(anyhow!("Ollama request failed: {e}"));
                }
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
            .map(std::string::ToString::to_string)
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

#[cfg(test)]
mod tests {
    use super::OllamaBackend;
    use crate::backend::InferenceBackend;

    #[tokio::test]
    async fn disabled_mock_fallback_returns_ollama_connection_error() {
        let backend = OllamaBackend::with_mock_fallback("http://127.0.0.1:1".to_string(), false);
        let result = backend.generate("model", "prompt", 8, None, None).await;

        assert!(result.is_err());
    }
}
