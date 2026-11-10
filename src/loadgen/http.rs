//! Minimal plain-HTTP JSON client for the node RPC (devnet load generation only).

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::core::types::CommitEvent;

/// Upper bound on one request; a node that does not answer within it counts as a transport
/// failure, not a rejection.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest SSE buffer kept while waiting for an event terminator.
const MAX_SSE_BUFFER: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("invalid request: {0}")]
    Request(String),
    #[error("transport: {0}")]
    Transport(String),
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("unexpected status {0}")]
    Status(StatusCode),
    #[error("malformed response: {0}")]
    Body(String),
}

#[derive(Clone)]
pub struct RpcClient {
    client: Client<HttpConnector, Full<Bytes>>,
}

impl Default for RpcClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RpcClient {
    pub fn new() -> Self {
        Self {
            client: Client::builder(TokioExecutor::new()).build_http(),
        }
    }

    /// GET a JSON body; any non-2xx status is an error.
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, HttpError> {
        let (status, body) = self.send(Method::GET, url, Bytes::new()).await?;
        if !status.is_success() {
            return Err(HttpError::Status(status));
        }
        serde_json::from_slice(&body).map_err(|e| HttpError::Body(e.to_string()))
    }

    /// POST a JSON body and return the status code (the body is read and discarded).
    pub async fn post_json<T: Serialize>(
        &self,
        url: &str,
        value: &T,
    ) -> Result<StatusCode, HttpError> {
        let body = serde_json::to_vec(value).map_err(|e| HttpError::Request(e.to_string()))?;
        let (status, _) = self.send(Method::POST, url, Bytes::from(body)).await?;
        Ok(status)
    }

    async fn send(
        &self,
        method: Method,
        url: &str,
        body: Bytes,
    ) -> Result<(StatusCode, Bytes), HttpError> {
        let request = Request::builder()
            .method(method)
            .uri(url)
            .header("content-type", "application/json")
            .body(Full::new(body))
            .map_err(|e| HttpError::Request(e.to_string()))?;
        let exchange = async {
            let response = self
                .client
                .request(request)
                .await
                .map_err(|e| HttpError::Transport(e.to_string()))?;
            let status = response.status();
            let bytes = response
                .into_body()
                .collect()
                .await
                .map_err(|e| HttpError::Transport(e.to_string()))?
                .to_bytes();
            Ok((status, bytes))
        };
        tokio::time::timeout(REQUEST_TIMEOUT, exchange)
            .await
            .map_err(|_| HttpError::Timeout(REQUEST_TIMEOUT))?
    }

    /// Stream `commit_block` events from `url` (`/events/blocks`) into `sink`, stamped with
    /// their arrival time, until the stream ends or the receiver is dropped.
    pub async fn stream_commits(
        &self,
        url: &str,
        sink: mpsc::UnboundedSender<(Instant, CommitEvent)>,
    ) -> Result<(), HttpError> {
        let request = Request::builder()
            .uri(url)
            .header("accept", "text/event-stream")
            .body(Full::new(Bytes::new()))
            .map_err(|e| HttpError::Request(e.to_string()))?;
        let response = tokio::time::timeout(REQUEST_TIMEOUT, self.client.request(request))
            .await
            .map_err(|_| HttpError::Timeout(REQUEST_TIMEOUT))?
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        if !response.status().is_success() {
            return Err(HttpError::Status(response.status()));
        }
        let mut body = response.into_body();
        let mut parser = SseParser::default();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| HttpError::Transport(e.to_string()))?;
            let Some(data) = frame.data_ref() else {
                continue;
            };
            let arrived = Instant::now();
            for event in parser.push(data)? {
                if sink.send((arrived, event)).is_err() {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

/// Incremental parser for the node's `commit_block` server-sent events.
#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    /// Feed bytes; return the complete commit events they finish.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<CommitEvent>, HttpError> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = find_terminator(&self.buffer) {
            let block: Vec<u8> = self.buffer.drain(..end + 2).collect();
            let text = String::from_utf8_lossy(&block);
            if let Some(event) = parse_event_block(&text)? {
                events.push(event);
            }
        }
        if self.buffer.len() > MAX_SSE_BUFFER {
            return Err(HttpError::Body("unterminated server-sent event".into()));
        }
        Ok(events)
    }
}

fn find_terminator(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|w| w == b"\n\n")
}

fn parse_event_block(text: &str) -> Result<Option<CommitEvent>, HttpError> {
    let mut name = None;
    let mut data = String::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push_str(value.trim_start());
        }
    }
    if name.as_deref() != Some("commit_block") || data.is_empty() {
        return Ok(None); // keep-alive comment or another event type
    }
    serde_json::from_str(&data)
        .map(Some)
        .map_err(|e| HttpError::Body(format!("commit event: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENT: &str = "event: commit_block\ndata: {\"block_hash\":\"ab\",\"timestamp_secs\":1,\"tx_count\":3,\"receipt_count\":1}\n\n";

    #[test]
    fn test_sse_parser_joins_split_frames() {
        let mut parser = SseParser::default();
        let (head, tail) = EVENT.split_at(20);
        assert!(parser.push(head.as_bytes()).unwrap().is_empty());
        let events = parser.push(tail.as_bytes()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tx_count, 3);
        assert_eq!(events[0].receipt_count, 1);
    }

    #[test]
    fn test_sse_parser_skips_keepalives_and_reads_several_events() {
        let mut parser = SseParser::default();
        let input = format!(":\n\n{EVENT}{EVENT}");
        assert_eq!(parser.push(input.as_bytes()).unwrap().len(), 2);
    }

    #[test]
    fn test_sse_parser_rejects_malformed_commit_data() {
        let mut parser = SseParser::default();
        let bad = "event: commit_block\ndata: {not json}\n\n";
        assert!(parser.push(bad.as_bytes()).is_err());
    }
}
