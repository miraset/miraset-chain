use super::{GenerationResponse, InferenceBackend, ModelInfo, mock_generate};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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
            Some(id) => {
                Ok(id.eq_ignore_ascii_case(model) || id.contains(model) || model.contains(&id))
            }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use serde::Deserialize;

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
        assert_eq!(parsed.model_device_type.as_deref(), Some("cuda"));
    }
}
