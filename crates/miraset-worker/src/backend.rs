/// Inference backend abstraction
///
/// Supports multiple backends: Ollama, vLLM, or custom implementations

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

    /// Mock inference fallback when Ollama is unavailable
    async fn mock_generate(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u64,
    ) -> Result<GenerationResponse> {
        tracing::info!("Using mock inference for model: {}", model);

        // Generate a simple mock response
        let mock_text = format!(
            "Mock inference response for prompt: '{}'. This is a simulated AI response \
            generated because Ollama is not available. In production, this would be \
            replaced with actual model output.",
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
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await;

        // Fallback to mock inference if Ollama is unavailable or model not found
        let ollama_response: OllamaResponse = match response {
            Ok(resp) if resp.status().is_success() => {
                resp.json().await?
            }
            Ok(resp) => {
                tracing::warn!(
                    "Ollama request failed: {} - falling back to mock inference",
                    resp.status()
                );
                return self.mock_generate(model, prompt, max_tokens).await;
            }
            Err(e) => {
                tracing::warn!(
                    "Ollama connection failed: {} - falling back to mock inference",
                    e
                );
                return self.mock_generate(model, prompt, max_tokens).await;
            }
        };

        // Tokenize the response (simple space-based for MVP)
        let tokens: Vec<String> = ollama_response.response
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

        let models = ollama_response.models
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
        // Mock response
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
        Ok(self.models
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

        let response = backend.generate(
            "llama2",
            "Hello",
            100,
            None,
            None,
        ).await.unwrap();

        assert!(!response.text.is_empty());
        assert!(response.token_count > 0);
    }

    #[tokio::test]
    async fn test_model_listing() {
        let backend = MockBackend::new(vec!["llama2".to_string(), "mistral".to_string()]);

        let models = backend.list_models().await.unwrap();
        assert_eq!(models.len(), 2);
    }
}
