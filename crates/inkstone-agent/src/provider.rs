use crate::runtime::Cancel;
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, future::Future, pin::Pin, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub context_tokens: usize,
    pub output_tokens: usize,
}
impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-6-astra".into(),
            context_tokens: 1_050_000,
            output_tokens: 128_000,
        }
    }
}
impl ModelConfig {
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.base_url).context("服务地址无效")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("服务地址必须为不含账号、查询参数的 HTTP(S) 地址");
        }
        if self.model.trim().is_empty() {
            bail!("请填写模型名称");
        }
        if self.output_tokens < 256
            || self.output_tokens > 131_072
            || self.context_tokens < self.output_tokens.saturating_add(2048)
            || self.context_tokens > 2_000_000
        {
            bail!("上下文容量须比输出上限至少多 2048，最大 200 万 token");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}
#[derive(Clone, Debug, Default)]
pub struct Completion {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub usage: Option<Value>,
    /// Complete Responses items, including encrypted reasoning, for this run only.
    pub output_items: Vec<Value>,
}
pub type CompletionFuture<'a> = Pin<Box<dyn Future<Output = Result<Completion>> + Send + 'a>>;
pub trait ModelProvider: Send + Sync {
    fn complete<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        cancel: &'a Cancel,
        emit: &'a mut (dyn FnMut(String) -> Result<()> + Send),
    ) -> CompletionFuture<'a>;
}
pub struct CompatibleProvider {
    pub config: ModelConfig,
    pub key: String,
}
impl ModelProvider for CompatibleProvider {
    fn complete<'a>(
        &'a self,
        messages: &'a [Value],
        tools: &'a [Value],
        cancel: &'a Cancel,
        emit: &'a mut (dyn FnMut(String) -> Result<()> + Send),
    ) -> CompletionFuture<'a> {
        Box::pin(async move {
            self.config.validate()?;
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(180))
                .redirect(reqwest::redirect::Policy::none())
                .build()?;
            let responses =
                reqwest::Url::parse(&self.config.base_url)?.host_str() == Some("api.openai.com");
            let mut body = json!({"model":self.config.model,"messages":messages,"stream":true,"max_completion_tokens":self.config.output_tokens});
            if !tools.is_empty() {
                body["tools"] = tools.into();
            }
            if responses {
                body = responses_body(&self.config, messages, tools)?;
            }
            let endpoint = if responses {
                "responses"
            } else {
                "chat/completions"
            };
            let mut request = client
                .post(format!(
                    "{}/{endpoint}",
                    self.config.base_url.trim_end_matches('/')
                ))
                .json(&body);
            if !self.key.is_empty() {
                request = request.bearer_auth(&self.key);
            }
            let response = tokio::select! {
                _ = cancel.wait() => bail!("已取消"),
                response = request.send() => response.map_err(|_| anyhow::anyhow!("连接模型服务失败或超时"))?,
            };
            if !response.status().is_success() {
                let status = response.status().as_u16();
                let mut stream = response.bytes_stream();
                let mut bytes = Vec::new();
                let read = async {
                    while let Some(chunk) = stream.next().await {
                        let chunk = chunk?;
                        if bytes.len() + chunk.len() > 16_384 {
                            break;
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    Ok::<_, reqwest::Error>(())
                };
                tokio::select! {
                    _ = cancel.wait() => bail!("已取消"),
                    _ = tokio::time::timeout(Duration::from_secs(5), read) => (),
                }
                bail!("{}", service_error(status, &bytes, &self.key));
            }
            let mut stream = response.bytes_stream();
            let mut decoder = SseDecoder::default();
            let mut completion = Collector::default();
            let mut response_completion = ResponsesCollector::default();
            'stream: loop {
                let next = tokio::select! {
                    _ = cancel.wait() => bail!("已取消"),
                    next = tokio::time::timeout(Duration::from_secs(45), stream.next()) => next.context("模型响应等待超时")?,
                };
                let Some(bytes) = next else {
                    break;
                };
                let bytes = bytes.map_err(|_| anyhow::anyhow!("模型响应中断"))?;
                for event in decoder.push(&bytes)? {
                    let delta = if responses {
                        response_completion.event(&event)?
                    } else {
                        completion.event(&event)?
                    };
                    if let Some(delta) = delta {
                        emit(delta)?;
                    }
                    if response_completion.done || event == "[DONE]" {
                        break 'stream;
                    }
                }
            }
            decoder.finish()?;
            if responses {
                response_completion.finish()
            } else {
                completion.finish()
            }
        })
    }
}

// Only structured diagnostic identifiers are displayed. Never echo arbitrary
// provider error text, which may contain credentials or submitted note content.
fn service_error(status: u16, bytes: &[u8], key: &str) -> String {
    let mut message = format!("模型服务返回 HTTP {status}");
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        for field in ["type", "code", "param"] {
            if let Some(value) = value["error"][field].as_str()
                && !value.is_empty()
                && value.len() <= 80
                && !value.contains("sk-")
                && (key.is_empty() || !value.contains(key))
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.[]".contains(&c))
            {
                message.push_str(&format!(" · {field}={value}"));
            }
        }
    }
    message
}

fn responses_body(config: &ModelConfig, messages: &[Value], tools: &[Value]) -> Result<Value> {
    let mut input = vec![];
    for message in messages {
        if let Some(items) = message["response_items"].as_array() {
            input.extend(items.iter().cloned());
        } else if message["role"] == "tool" {
            input.push(json!({"type":"function_call_output", "call_id":message["tool_call_id"], "output":message["content"]}));
        } else {
            input.push(json!({"role":message["role"], "content":message["content"]}));
            if let Some(calls) = message["tool_calls"].as_array() {
                for call in calls {
                    input.push(json!({"type":"function_call", "call_id":call["id"], "name":call["function"]["name"], "arguments":call["function"]["arguments"]}));
                }
            }
        }
    }
    let tools: Vec<_> = tools
        .iter()
        .map(|tool| {
            let mut function = tool["function"].clone();
            function["type"] = "function".into();
            function["strict"] = false.into();
            function
        })
        .collect();
    Ok(
        json!({"model":config.model,"input":input,"tools":tools,"stream":true,
        "store":false,"include":["reasoning.encrypted_content"],"max_output_tokens":config.output_tokens}),
    )
}

#[derive(Default)]
struct ResponsesCollector {
    result: Completion,
    done: bool,
    size: usize,
}
impl ResponsesCollector {
    fn event(&mut self, event: &str) -> Result<Option<String>> {
        self.size += event.len();
        if self.size > 8 * 1024 * 1024 {
            bail!("模型响应过大");
        }
        let value: Value = serde_json::from_str(event).context("流式响应不是有效 JSON")?;
        match value["type"].as_str() {
            Some("response.output_text.delta") => {
                let text = value["delta"].as_str().context("响应缺少正文 delta")?;
                self.result.text.push_str(text);
                Ok(Some(text.into()))
            }
            Some("response.completed") => {
                let response = &value["response"];
                if response["status"] != "completed" {
                    bail!("响应未正常完成，未执行工具");
                }
                self.result.output_items = response["output"]
                    .as_array()
                    .context("响应缺少 output")?
                    .clone();
                let mut ids = std::collections::BTreeSet::new();
                for item in &self.result.output_items {
                    if item["type"] != "function_call" {
                        continue;
                    }
                    let id = item["call_id"].as_str().context("工具调用缺少 call_id")?;
                    let name = item["name"].as_str().context("工具调用缺少 name")?;
                    let arguments = item["arguments"]
                        .as_str()
                        .context("工具调用缺少 arguments")?;
                    if id.is_empty() || name.is_empty() || !ids.insert(id) || ids.len() > 40 {
                        bail!("工具调用身份无效或调用过多");
                    }
                    if !serde_json::from_str::<Value>(arguments)
                        .context("工具参数不完整")?
                        .is_object()
                    {
                        bail!("工具参数必须为对象");
                    }
                    self.result.calls.push(ToolCall {
                        id: id.into(),
                        name: name.into(),
                        arguments: arguments.into(),
                    });
                }
                self.result.usage = response.get("usage").filter(|v| !v.is_null()).cloned();
                self.done = true;
                Ok(None)
            }
            Some("response.failed" | "response.incomplete" | "error") => {
                bail!("模型响应失败或被截断，未执行工具")
            }
            _ => Ok(None),
        }
    }
    fn finish(self) -> Result<Completion> {
        if !self.done {
            bail!("响应被截断或未正常完成，未执行工具");
        }
        Ok(self.result)
    }
}

/// Buffers bytes, not UTF-8 strings: TCP may split a Chinese character anywhere.
#[derive(Default)]
pub struct SseDecoder {
    bytes: Vec<u8>,
    data: Vec<String>,
}
impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        self.bytes.extend_from_slice(bytes);
        if self.bytes.len() > 1024 * 1024 {
            bail!("流式事件超过大小限制");
        }
        let mut events = vec![];
        while let Some(end) = self.bytes.iter().position(|b| *b == b'\n') {
            let line = self.bytes.drain(..=end).collect::<Vec<_>>();
            let line = std::str::from_utf8(&line)?.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(self.data.join("\n"));
                    self.data.clear();
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).into());
                if self.data.iter().map(String::len).sum::<usize>() > 1024 * 1024 {
                    bail!("流式事件超过大小限制");
                }
            }
        }
        Ok(events)
    }
    pub fn finish(&self) -> Result<()> {
        if !self.bytes.is_empty() || !self.data.is_empty() {
            bail!("流式响应不完整");
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct Collector {
    result: Completion,
    calls: BTreeMap<usize, ToolCall>,
    done: bool,
    finish_reason: Option<String>,
    size: usize,
}
impl Collector {
    pub fn event(&mut self, event: &str) -> Result<Option<String>> {
        if self.done {
            bail!("结束标记后仍收到数据");
        }
        if event == "[DONE]" {
            self.done = true;
            return Ok(None);
        }
        self.size += event.len();
        if self.size > 8 * 1024 * 1024 {
            bail!("模型响应过大");
        }
        let value: Value = serde_json::from_str(event).context("流式响应不是有效 JSON")?;
        if value.get("error").is_some() {
            bail!("模型服务在响应过程中报告错误");
        }
        if value.get("usage").is_some_and(|u| !u.is_null()) {
            self.result.usage = Some(value["usage"].clone());
        }
        let Some(choices) = value["choices"].as_array() else {
            bail!("响应缺少 choices");
        };
        let mut text = None;
        for choice in choices {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                continue;
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                self.finish_reason = Some(reason.into());
            }
            let delta = &choice["delta"];
            if let Some(chunk) = delta["content"].as_str() {
                self.result.text.push_str(chunk);
                text = Some(chunk.to_owned());
            }
            if let Some(calls) = delta["tool_calls"].as_array() {
                for part in calls {
                    let index = part["index"].as_u64().context("工具调用缺少 index")? as usize;
                    if index > 39 {
                        bail!("单轮工具调用过多");
                    }
                    let call = self.calls.entry(index).or_default();
                    if let Some(id) = part["id"].as_str() {
                        call.id.push_str(id);
                    }
                    if let Some(name) = part["function"]["name"].as_str() {
                        call.name.push_str(name);
                    }
                    if let Some(args) = part["function"]["arguments"].as_str() {
                        call.arguments.push_str(args);
                    }
                }
            }
        }
        Ok(text)
    }
    pub fn finish(mut self) -> Result<Completion> {
        if !self.done || !matches!(self.finish_reason.as_deref(), Some("stop" | "tool_calls")) {
            bail!("响应被截断或未正常完成，未执行工具");
        }
        let mut ids = std::collections::BTreeSet::new();
        for call in self.calls.into_values() {
            if call.id.is_empty() || call.name.is_empty() || !ids.insert(call.id.clone()) {
                bail!("工具调用身份无效");
            }
            let args: Value = serde_json::from_str(&call.arguments).context("工具参数不完整")?;
            if !args.is_object() {
                bail!("工具参数必须为对象");
            }
            self.result.calls.push(call);
        }
        Ok(self.result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn responses_stream_preserves_reasoning_and_tool_result_pairing() {
        let items = json!([
            {"type":"reasoning","id":"r1","summary":[],"encrypted_content":"opaque"},
            {"type":"function_call","id":"fc1","call_id":"call1","name":"read_note","arguments":"{\"path\":\"a.md\"}"}
        ]);
        let wire = format!(
            "data: {}\n\ndata: {}\n\n",
            json!({"type":"response.output_text.delta","delta":"中文😀"}),
            json!({"type":"response.completed","response":{"status":"completed","output":items,"usage":{"output_tokens":12}}})
        );
        for size in 1..wire.len() {
            let mut decoder = SseDecoder::default();
            let mut collector = ResponsesCollector::default();
            for chunk in wire.as_bytes().chunks(size) {
                for event in decoder.push(chunk).unwrap() {
                    collector.event(&event).unwrap();
                }
            }
            decoder.finish().unwrap();
            let result = collector.finish().unwrap();
            assert_eq!(result.text, "中文😀");
            assert_eq!(result.calls[0].id, "call1");
            assert_eq!(result.output_items, items.as_array().unwrap().clone());
            let body = responses_body(
                &ModelConfig::default(),
                &[
                    json!({"role":"user","content":"读取"}),
                    json!({"role":"assistant","response_items":result.output_items}),
                    json!({"role":"tool","tool_call_id":"call1","content":"synthetic result"}),
                ],
                &crate::tools::definitions(),
            )
            .unwrap();
            assert_eq!(body["input"][1]["encrypted_content"], "opaque");
            assert_eq!(body["input"][3]["type"], "function_call_output");
            assert_eq!(body["input"][3]["call_id"], "call1");
            assert_eq!(body["store"], false);
            assert_eq!(body["tools"][0]["type"], "function");
            assert_eq!(body["tools"][0]["strict"], false);
            assert!(body.get("max_completion_tokens").is_none());
        }
    }
    #[test]
    fn responses_reject_incomplete_and_invalid_calls() {
        let mut collector = ResponsesCollector::default();
        collector.event(&json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"a","name":"read_note","arguments":"{}"}}).to_string()).unwrap();
        assert!(collector.finish().is_err());
        for event in [
            json!({"type":"response.incomplete"}),
            json!({"type":"response.failed"}),
            json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"function_call","call_id":"a","name":"read_note","arguments":"{"}]}}),
        ] {
            assert!(
                ResponsesCollector::default()
                    .event(&event.to_string())
                    .is_err()
            );
        }
    }
    #[test]
    fn error_diagnostics_do_not_echo_credentials_or_note_content() {
        let body = json!({"error":{"message":"secret-key private note", "type":"invalid_request_error","param":"max_output_tokens","code":"secret-key"}}).to_string();
        let error = service_error(400, body.as_bytes(), "secret-key");
        assert!(error.contains("param=max_output_tokens"));
        assert!(error.contains("invalid_request_error"));
        assert!(!error.contains("secret-key"));
        assert!(!error.contains("private note"));
    }
    #[test]
    fn sse_every_byte_boundary_and_interleaved_calls() {
        let wire = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"中文😀\"},\"finish_reason\":null}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"b\",\"function\":{\"name\":\"read\",\"arguments\":\"{\"}},{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"list\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        for size in 1..wire.len() {
            let mut decoder = SseDecoder::default();
            let mut collector = Collector::default();
            for bytes in wire.as_bytes().chunks(size) {
                for event in decoder.push(bytes).unwrap() {
                    collector.event(&event).unwrap();
                }
            }
            decoder.finish().unwrap();
            let result = collector.finish().unwrap();
            assert_eq!(result.text, "中文😀");
            assert_eq!(result.calls[0].id, "a");
            assert_eq!(result.calls[1].arguments, "{}");
        }
    }
    #[test]
    fn rejects_overflowing_configuration_without_panicking() {
        let config = ModelConfig {
            model: "test".into(),
            output_tokens: usize::MAX,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
    #[test]
    fn truncated_call_never_finishes() {
        let mut c = Collector::default();
        c.event(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"read","arguments":"{"}}]},"finish_reason":"length"}]}"#).unwrap();
        c.event("[DONE]").unwrap();
        assert!(c.finish().is_err());
    }
}

#[cfg(test)]
pub(crate) mod http_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    pub(crate) fn server(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = vec![];
            let mut buffer = [0; 1024];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let size = head
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .and_then(|n| n.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let _ = stream.write_all(response.as_bytes());
            String::from_utf8(request).unwrap()
        });
        (url, handle)
    }
    #[test]
    fn real_http_stream_and_errors_are_redacted() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let (url, server) = server("200 OK", body);
        let provider = CompatibleProvider {
            config: ModelConfig {
                base_url: url,
                model: "test".into(),
                ..Default::default()
            },
            key: "test-secret".into(),
        };
        let mut deltas = String::new();
        let result = runtime
            .block_on(provider.complete(
                &[json!({"role":"user","content":"hello"})],
                &[],
                &Cancel::default(),
                &mut |text| {
                    deltas.push_str(&text);
                    Ok(())
                },
            ))
            .unwrap();
        assert_eq!(result.text, "你好");
        assert_eq!(deltas, "你好");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions"));
        assert!(request.contains("Bearer test-secret"));
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["max_completion_tokens"], provider.config.output_tokens);
        assert!(body.get("max_tokens").is_none());
        let (url, server) = self::server(
            "429 Too Many Requests",
            "private-provider-error test-secret",
        );
        let provider = CompatibleProvider {
            config: ModelConfig {
                base_url: url,
                model: "test".into(),
                ..Default::default()
            },
            key: "test-secret".into(),
        };
        let error = runtime
            .block_on(provider.complete(&[], &[], &Cancel::default(), &mut |_| Ok(())))
            .unwrap_err()
            .to_string();
        server.join().unwrap();
        assert!(error.contains("429"));
        assert!(!error.contains("secret"));
    }
    #[test]
    fn cancel_interrupts_pending_http_without_waiting_for_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(500));
        });
        let provider = CompatibleProvider {
            config: ModelConfig {
                base_url: url,
                model: "test".into(),
                ..Default::default()
            },
            key: String::new(),
        };
        let cancel = Cancel::default();
        let other = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            other.cancel();
        });
        let start = std::time::Instant::now();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(provider.complete(&[], &[], &cancel, &mut |_| Ok(())));
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }
}
