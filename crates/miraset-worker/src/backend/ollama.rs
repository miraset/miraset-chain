use super::{GenerationResponse, InferenceBackend, ModelInfo, mock_generate};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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
