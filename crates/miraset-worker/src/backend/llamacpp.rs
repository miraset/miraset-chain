use super::{GenerationResponse, InferenceBackend, ModelInfo, mock_generate};
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

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
        if let Some(server_model) = self.server_model_id().await
            && !server_model.eq_ignore_ascii_case(model)
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

        #[derive(serde::Serialize)]
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
        let token_count = lcpp_response
            .tokens_predicted
            .unwrap_or(tokens.len() as u64);

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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use serde::Deserialize;

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
        let body =
            r#"{"model_name": "qwen2.5-7b-instruct-q5_k_m", "model_path": "/models/qwen.gguf"}"#;
        #[derive(Deserialize)]
        struct Props {
            #[serde(default)]
            model_name: Option<String>,
            #[serde(default)]
            model_path: Option<String>,
        }
        let parsed: Props = serde_json::from_str(body).unwrap();
        assert_eq!(
            parsed.model_name.as_deref(),
            Some("qwen2.5-7b-instruct-q5_k_m")
        );
        // model_name takes precedence over model_path in our code.
        assert_eq!(
            parsed.model_name.or(parsed.model_path).unwrap(),
            "qwen2.5-7b-instruct-q5_k_m"
        );
    }
}
