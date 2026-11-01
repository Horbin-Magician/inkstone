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
    pub tools: bool,
}
impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "默认模型".into(),
            base_url: "http://localhost:11434/v1".into(),
            model: String::new(),
            context_tokens: 32768,
            output_tokens: 4096,
            tools: true,
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
            let mut body = json!({"model":self.config.model,"messages":messages,"stream":true,"max_tokens":self.config.output_tokens});
            if !tools.is_empty() {
                body["tools"] = tools.into();
            }
            let mut request = client
                .post(format!(
                    "{}/chat/completions",
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
                bail!("模型服务返回 HTTP {}", response.status().as_u16());
            }
            let mut stream = response.bytes_stream();
            let mut decoder = SseDecoder::default();
            let mut completion = Collector::default();
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
                    if let Some(delta) = completion.event(&event)? {
                        emit(delta)?;
                    }
                    if event == "[DONE]" {
                        break 'stream;
                    }
                }
            }
            decoder.finish()?;
            completion.finish()
        })
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
