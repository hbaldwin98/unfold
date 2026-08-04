use std::collections::HashSet;

use futures_util::StreamExt;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    auth, secrets,
    settings::{Protocol, Provider, Settings},
};

const CHATGPT_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const INSTRUCTIONS: &str = r#"You help learners solve problems themselves by teaching through worked examples.

Do not simply solve the learner's exact target problem or reveal its final answer. Instead:
1. Briefly restate what the learner is trying to find.
2. List the concepts, formulas, facts, or other ingredients they will need.
3. Give a short sequence of steps they can apply to their target problem. Leave the decisive calculation or conclusion for the learner.
4. Create a closely analogous example with different values or details, and solve that example completely step by step.
5. End with one useful check, hint, or question that helps the learner continue their own problem.
6. When web search is used, cite sources next to factual claims and include useful source links.

Clearly label guidance for the learner's problem separately from the solved worked example. Use Markdown and LaTeX notation where it improves clarity. Prefer $...$ for inline mathematics and $$...$$ for display mathematics. Be concise, direct, and educational."#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateRequest {
    pub prompt: String,
    pub web_search: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "event", content = "data")]
pub enum ResponseEvent {
    Started,
    TextDelta { delta: String },
    Source { title: String, url: String },
    Completed,
    Cancelled,
    Failed { message: String },
}

pub async fn generate(
    settings: Settings,
    request: GenerateRequest,
    on_event: Channel<ResponseEvent>,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err("Enter a problem or topic first".to_owned());
    }
    if request.web_search
        && settings.provider == Provider::Compatible
        && settings.protocol == Protocol::ChatCompletions
    {
        return Err("Web search is unavailable with Chat Completions".to_owned());
    }

    let _ = on_event.send(ResponseEvent::Started);
    let response = send_request(&settings, &request).await?;
    stream_response(response, settings.protocol, on_event, cancellation).await
}

async fn send_request(settings: &Settings, request: &GenerateRequest) -> Result<Response, String> {
    let client = Client::new();
    let prompt = request.prompt.trim();

    match settings.provider {
        Provider::Chatgpt => {
            let (access_token, account_id) = auth::valid_access_token().await?;
            let mut body = json!({
                "model": settings.model,
                "instructions": INSTRUCTIONS,
                "input": [{
                    "role": "user",
                    "content": [{ "type": "input_text", "text": prompt }]
                }],
                "stream": true,
                "store": false
            });
            if request.web_search {
                body["tools"] = json!([{ "type": "web_search" }]);
            }

            let mut builder = client
                .post(CHATGPT_RESPONSES_URL)
                .bearer_auth(access_token)
                .header("Accept", "text/event-stream")
                .header("originator", "worked_examples")
                .header("session-id", Uuid::new_v4().to_string())
                .header("User-Agent", "worked-examples/0.1.2")
                .json(&body);
            if let Some(account_id) = account_id {
                builder = builder.header("ChatGPT-Account-Id", account_id);
            }
            checked(builder.send().await).await
        }
        Provider::Compatible => {
            let api_key = secrets::load_api_key()?;
            let path = match settings.protocol {
                Protocol::Responses => "responses",
                Protocol::ChatCompletions => "chat/completions",
            };
            let url = endpoint_url(&settings.base_url, path);
            let body = match settings.protocol {
                Protocol::Responses => {
                    let mut body = json!({
                        "model": settings.model,
                        "instructions": INSTRUCTIONS,
                        "input": [{
                            "role": "user",
                            "content": [{ "type": "input_text", "text": prompt }]
                        }],
                        "stream": true
                    });
                    if request.web_search {
                        body["tools"] = json!([{ "type": "web_search" }]);
                    }
                    body
                }
                Protocol::ChatCompletions => json!({
                    "model": settings.model,
                    "messages": [
                        { "role": "system", "content": INSTRUCTIONS },
                        { "role": "user", "content": prompt }
                    ],
                    "stream": true
                }),
            };

            let mut builder = client
                .post(url)
                .header("Accept", "text/event-stream")
                .json(&body);
            if let Some(api_key) = api_key {
                builder = builder.bearer_auth(api_key);
            }
            checked(builder.send().await).await
        }
    }
}

async fn checked(result: Result<Response, reqwest::Error>) -> Result<Response, String> {
    let response =
        result.map_err(|error| format!("Could not reach the model provider: {error}"))?;
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let detail = response
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(500)
        .collect::<String>();
    if detail.is_empty() {
        Err(format!("The model provider returned {status}"))
    } else {
        Err(format!("The model provider returned {status}: {detail}"))
    }
}

async fn stream_response(
    response: Response,
    protocol: Protocol,
    on_event: Channel<ResponseEvent>,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let mut stream = response.bytes_stream();
    let mut pending = String::new();
    let mut sources = HashSet::new();

    loop {
        let item = tokio::select! {
            _ = cancellation.cancelled() => {
                let _ = on_event.send(ResponseEvent::Cancelled);
                return Ok(());
            }
            item = stream.next() => item,
        };

        let Some(chunk) = item else {
            break;
        };
        let chunk = chunk.map_err(|error| format!("The response stream failed: {error}"))?;
        pending.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(index) = pending.find('\n') {
            let line = pending[..index].trim_end_matches('\r').to_owned();
            pending.drain(..=index);
            if let Some(data) = line.strip_prefix("data:").map(str::trim) {
                if data == "[DONE]" {
                    let _ = on_event.send(ResponseEvent::Completed);
                    return Ok(());
                }
                for event in parse_data(data, protocol) {
                    match &event {
                        ResponseEvent::Source { url, .. } if !sources.insert(url.clone()) => {}
                        ResponseEvent::Failed { .. } => {
                            let _ = on_event.send(event);
                            return Ok(());
                        }
                        _ => {
                            let _ = on_event.send(event);
                        }
                    }
                }
            }
        }
    }

    if !pending.trim().is_empty() {
        for event in parse_data(pending.trim().trim_start_matches("data:").trim(), protocol) {
            let _ = on_event.send(event);
        }
    }
    let _ = on_event.send(ResponseEvent::Completed);
    Ok(())
}

fn parse_data(data: &str, protocol: Protocol) -> Vec<ResponseEvent> {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };

    match protocol {
        Protocol::ChatCompletions => value
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
            .map(|delta| {
                vec![ResponseEvent::TextDelta {
                    delta: delta.to_owned(),
                }]
            })
            .unwrap_or_default(),
        Protocol::Responses => parse_responses_event(&value),
    }
}

fn parse_responses_event(value: &Value) -> Vec<ResponseEvent> {
    if value.get("type").and_then(Value::as_str) == Some("response.output_text.delta") {
        return value
            .get("delta")
            .and_then(Value::as_str)
            .map(|delta| {
                vec![ResponseEvent::TextDelta {
                    delta: delta.to_owned(),
                }]
            })
            .unwrap_or_default();
    }

    if matches!(
        value.get("type").and_then(Value::as_str),
        Some("response.failed" | "error")
    ) {
        let message = value
            .pointer("/response/error/message")
            .or_else(|| value.pointer("/error/message"))
            .and_then(Value::as_str)
            .unwrap_or("The provider could not complete the response");
        return vec![ResponseEvent::Failed {
            message: message.to_owned(),
        }];
    }

    let mut events = Vec::new();
    collect_sources(value, &mut events);
    events
}

fn collect_sources(value: &Value, events: &mut Vec<ResponseEvent>) {
    match value {
        Value::Object(object) => {
            if let Some(url) = object.get("url").and_then(Value::as_str)
                && (url.starts_with("https://") || url.starts_with("http://"))
            {
                let title = object
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(url)
                    .to_owned();
                events.push(ResponseEvent::Source {
                    title,
                    url: url.to_owned(),
                });
            }
            for child in object.values() {
                collect_sources(child, events);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_sources(child, events);
            }
        }
        _ => {}
    }
}

fn endpoint_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_responses_text_delta() {
        let events = parse_data(
            r#"{"type":"response.output_text.delta","delta":"Step 1"}"#,
            Protocol::Responses,
        );
        assert!(matches!(
            events.as_slice(),
            [ResponseEvent::TextDelta { delta }] if delta == "Step 1"
        ));
    }

    #[test]
    fn parses_chat_completion_delta() {
        let events = parse_data(
            r#"{"choices":[{"delta":{"content":"Answer"}}]}"#,
            Protocol::ChatCompletions,
        );
        assert!(matches!(
            events.as_slice(),
            [ResponseEvent::TextDelta { delta }] if delta == "Answer"
        ));
    }

    #[test]
    fn appends_v1_only_when_needed() {
        assert_eq!(
            endpoint_url("http://localhost:11434", "responses"),
            "http://localhost:11434/v1/responses"
        );
        assert_eq!(
            endpoint_url("http://localhost:11434/v1/", "responses"),
            "http://localhost:11434/v1/responses"
        );
    }
}
