mod capacity;
mod maintenance;
use super::*;
pub use capacity::CloudCapacity;
pub use maintenance::MaintenanceLock;
use reqwest::{
    Method, StatusCode, Url,
    blocking::{Body, Client, Response},
    header,
};
use std::{
    io::{Read, Seek},
    time::Duration,
};

pub(super) fn parse_url(settings: &Settings) -> Result<Url> {
    let mut root = Url::parse(settings.url.trim()).context("请输入完整的 WebDAV 目录地址")?;
    ensure!(
        matches!(root.scheme(), "http" | "https")
            && root.host_str().is_some()
            && root.username().is_empty()
            && root.password().is_none()
            && root.query().is_none()
            && root.fragment().is_none(),
        "WebDAV 地址必须为 HTTP(S) 目录，账号请填写在用户名栏"
    );
    if !root.path().ends_with('/') {
        root.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("无效目录地址"))?
            .push("");
    }
    Ok(root)
}

pub struct WebDav {
    client: Client,
    root: Url,
    username: String,
    password: String,
}
impl WebDav {
    pub fn new(settings: &Settings, password: &str) -> Result<Self> {
        let root = parse_url(settings)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Inkstone/0.1 WebDAV")
            .build()
            .context("无法初始化 WebDAV 客户端")?;
        Ok(Self {
            client,
            root,
            username: settings.username.clone(),
            password: password.into(),
        })
    }
    pub fn identity(&self) -> String {
        format!("{}\n{}", self.root, self.username)
    }
    fn request(&self, method: Method, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .request(method, self.root.join(path).expect("fixed relative URL"))
            .basic_auth(&self.username, Some(&self.password))
    }
    fn send(&self, request: reqwest::blocking::RequestBuilder) -> Result<Response> {
        // Do not include reqwest's URL-bearing errors in UI/logs.
        request.send().map_err(|e| {
            anyhow::anyhow!(if e.is_timeout() {
                "WebDAV 请求超时，请检查网络后重试"
            } else if e.is_connect() {
                "无法连接 WebDAV，请检查地址、网络与 TLS 证书"
            } else {
                "WebDAV 网络请求失败，请重试"
            })
        })
    }
    fn status(response: Response) -> Result<Response> {
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                bail!("WebDAV 认证失败或无访问权限")
            }
            StatusCode::PRECONDITION_FAILED => bail!("其他设备已更新云端，请重新同步"),
            code if code.is_success() => Ok(response),
            code => bail!("WebDAV 返回 HTTP {}", code.as_u16()),
        }
    }
    pub fn test_connection(&self) -> Result<()> {
        let response = Self::status(self.send(self.request(Method::from_bytes(b"PROPFIND")?, "")
            .header("Depth", "0").header(header::CONTENT_TYPE, "application/xml")
            .body("<?xml version=\"1.0\"?><d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/></d:prop></d:propfind>"))?)?;
        ensure!(
            response.status() == StatusCode::MULTI_STATUS,
            "服务器未返回 WebDAV 目录响应"
        );
        Ok(())
    }
    fn check_legacy_directory(&self) -> Result<()> {
        let response = self.send(self.request(Method::GET, "inkstone-v1/manifest.json"))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        Self::status(response)?;
        bail!(
            "检测到旧同步目录 inkstone-v1：请先更新所有设备，再将远端 inkstone-v1 文件夹重命名为 inkstone；若两个目录同时存在，请先备份并核对数据，不要直接覆盖"
        )
    }
    pub fn prepare(&self) -> Result<()> {
        self.test_connection()?;
        self.check_legacy_directory()?;
        for path in ["inkstone/", "inkstone/objects/"] {
            let response = self.send(self.request(Method::from_bytes(b"MKCOL")?, path))?;
            if response.status() != StatusCode::METHOD_NOT_ALLOWED {
                Self::status(response)?;
            }
        }
        Ok(())
    }
    fn body(response: Response, limit: u64) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        Self::body_to(response, limit, &mut bytes)?;
        Ok(bytes)
    }
    fn body_to(response: Response, limit: u64, output: &mut dyn Write) -> Result<u64> {
        ensure!(
            response.content_length().is_none_or(|n| n <= limit),
            "WebDAV 文件超过大小限制"
        );
        copy_limited(response, output, limit)
    }
}
impl Remote for WebDav {
    fn manifest(&self) -> Result<(Manifest, Option<String>)> {
        self.check_legacy_directory()?;
        let response = self.send(
            self.request(Method::GET, "inkstone/manifest.json")
                .header(header::CACHE_CONTROL, "no-cache"),
        )?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok((Manifest::default(), None));
        }
        let response = Self::status(response)?;
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.starts_with('"') && v.ends_with('"'))
            .context("WebDAV 服务器未提供强 ETag，无法安全同步")?
            .to_owned();
        let manifest: Manifest = serde_json::from_slice(&Self::body(response, MAX_MANIFEST_BYTES)?)
            .context("云端同步清单损坏或格式不兼容")?;
        validate_manifest(&manifest)?;
        Ok((manifest, Some(etag)))
    }
    fn download(&self, digest: &str) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.download_to(digest, &mut bytes)?;
        Ok(bytes)
    }
    fn download_to(&self, digest: &str, output: &mut dyn Write) -> Result<u64> {
        ensure!(valid_hash(digest), "无效的对象校验值");
        Self::body_to(
            Self::status(
                self.send(self.request(Method::GET, &format!("inkstone/objects/{digest}")))?,
            )?,
            MAX_FILE_BYTES,
            output,
        )
    }
    fn upload(&self, digest: &str, bytes: &[u8]) -> Result<()> {
        ensure!(
            valid_hash(digest) && hash(bytes) == digest,
            "上传内容校验失败"
        );
        self.put_object(digest, Body::from(bytes.to_vec()))
    }
    fn upload_file(&self, digest: &str, mut file: fs::File) -> Result<()> {
        ensure!(valid_hash(digest), "无效的对象校验值");
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "上传目标不是普通文件");
        ensure!(metadata.len() <= MAX_FILE_BYTES, "上传文件超过大小限制");
        file.rewind()?;
        let mut sink = DigestSink(Sha256::new());
        let bytes = copy_limited(&mut file, &mut sink, MAX_FILE_BYTES)?;
        ensure!(
            format!("{:x}", sink.0.finalize()) == digest,
            "上传内容校验失败"
        );
        file.rewind()?;
        self.put_object(digest, Body::sized(file, bytes))
    }
    fn publish(&self, manifest: &Manifest, revision: Option<&str>) -> Result<()> {
        self.publish_conditionally(manifest, revision, None)
    }
}

impl WebDav {
    fn publish_conditionally(
        &self,
        manifest: &Manifest,
        revision: Option<&str>,
        lock_condition: Option<header::HeaderValue>,
    ) -> Result<()> {
        validate_manifest(manifest)?;
        let bytes = serde_json::to_vec(manifest)?;
        ensure!(
            bytes.len() as u64 <= MAX_MANIFEST_BYTES,
            "同步清单超过大小限制"
        );
        let request = self
            .request(Method::PUT, "inkstone/manifest.json")
            .header(header::CONTENT_TYPE, "application/json")
            .body(bytes);
        let request = match revision {
            Some(etag) => request.header(header::IF_MATCH, etag),
            None => request.header(header::IF_NONE_MATCH, "*"),
        };
        let request = if let Some(condition) = lock_condition {
            request
                .header("If", condition)
                .header(header::CACHE_CONTROL, "no-cache")
        } else {
            request
        };
        let response = Self::status(self.send(request)?)?;
        ensure!(
            matches!(
                response.status(),
                StatusCode::OK | StatusCode::CREATED | StatusCode::NO_CONTENT
            ),
            "服务器未确认清单发布"
        );
        Ok(())
    }
}

impl WebDav {
    fn put_object(&self, digest: &str, body: Body) -> Result<()> {
        let response = self.send(
            self.request(Method::PUT, &format!("inkstone/objects/{digest}"))
                .header(header::IF_NONE_MATCH, "*")
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(body),
        )?;
        if response.status() == StatusCode::PRECONDITION_FAILED {
            let mut sink = DigestSink(Sha256::new());
            self.download_to(digest, &mut sink)?;
            ensure!(
                format!("{:x}", sink.0.finalize()) == digest,
                "已有云端对象校验失败"
            );
        } else {
            Self::status(response)?;
        }
        Ok(())
    }
}

fn copy_limited(mut input: impl Read, output: &mut dyn Write, limit: u64) -> Result<u64> {
    let mut buffer = [0u8; 64 * 1024];
    let mut copied = 0u64;
    loop {
        let count = input.read(&mut buffer).context("读取 WebDAV 响应失败")?;
        if count == 0 {
            return Ok(copied);
        }
        ensure!(
            count as u64 <= limit.saturating_sub(copied),
            "WebDAV 文件超过大小限制"
        );
        output
            .write_all(&buffer[..count])
            .context("写入 WebDAV 下载目标失败")?;
        copied += count as u64;
    }
}
struct DigestSink(Sha256);
impl Write for DigestSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
mod stream_tests {
    use super::*;
    #[test]
    fn bounded_stream_handles_short_writes_limits_and_destination_failure() -> Result<()> {
        struct Sink {
            bytes: Vec<u8>,
            max_chunk: usize,
            fail: bool,
        }
        impl Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.fail {
                    return Err(io::Error::other("disk full"));
                }
                self.max_chunk = self.max_chunk.max(bytes.len());
                let count = bytes.len().min(97);
                self.bytes.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let input = vec![137u8; 200_000];
        let mut sink = Sink {
            bytes: vec![],
            max_chunk: 0,
            fail: false,
        };
        assert_eq!(copy_limited(input.as_slice(), &mut sink, 200_000)?, 200_000);
        assert_eq!(sink.bytes, input);
        assert!(sink.max_chunk <= 64 * 1024);
        assert!(copy_limited(input.as_slice(), &mut io::sink(), 199_999).is_err());
        sink.fail = true;
        assert!(copy_limited(input.as_slice(), &mut sink, 200_000).is_err());
        Ok(())
    }
}
