use super::{GenerationResponse, InferenceBackend, ModelInfo};
use anyhow::Result;
use async_trait::async_trait;

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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[tokio::test]
    async fn test_mock_backend() {
        let backend = MockBackend::new(vec!["llama2".to_string()]);

        let response = backend
            .generate("llama2", "Hello", 100, None, None)
            .await
            .unwrap();

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
