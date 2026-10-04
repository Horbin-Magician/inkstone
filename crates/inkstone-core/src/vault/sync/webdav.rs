use super::*;
use reqwest::{
    Method, StatusCode, Url,
    blocking::{Client, Response},
    header,
};
use std::{io::Read, time::Duration};

pub struct WebDav {
    client: Client,
    root: Url,
    username: String,
    password: String,
}
impl WebDav {
    pub fn new(settings: &Settings, password: &str) -> Result<Self> {
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
    pub fn prepare(&self) -> Result<()> {
        self.test_connection()?;
        for path in ["inkstone-v1/", "inkstone-v1/objects/"] {
            let response = self.send(self.request(Method::from_bytes(b"MKCOL")?, path))?;
            if response.status() != StatusCode::METHOD_NOT_ALLOWED {
                Self::status(response)?;
            }
        }
        Ok(())
    }
    fn body(response: Response, limit: u64) -> Result<Vec<u8>> {
        ensure!(
            response.content_length().is_none_or(|n| n <= limit),
            "WebDAV 文件超过大小限制"
        );
        let mut bytes = Vec::new();
        response
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .context("读取 WebDAV 响应失败")?;
        ensure!(bytes.len() as u64 <= limit, "WebDAV 文件超过大小限制");
        Ok(bytes)
    }
}
impl Remote for WebDav {
    fn manifest(&self) -> Result<(Manifest, Option<String>)> {
        let response = self.send(
            self.request(Method::GET, "inkstone-v1/manifest.json")
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
        ensure!(valid_hash(digest), "无效的对象校验值");
        Self::body(
            Self::status(
                self.send(self.request(Method::GET, &format!("inkstone-v1/objects/{digest}")))?,
            )?,
            MAX_FILE_BYTES,
        )
    }
    fn upload(&self, digest: &str, bytes: &[u8]) -> Result<()> {
        ensure!(
            valid_hash(digest) && hash(bytes) == digest,
            "上传内容校验失败"
        );
        let response = self.send(
            self.request(Method::PUT, &format!("inkstone-v1/objects/{digest}"))
                .header(header::IF_NONE_MATCH, "*")
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(bytes.to_vec()),
        )?;
        if response.status() == StatusCode::PRECONDITION_FAILED {
            ensure!(
                hash(&self.download(digest)?) == digest,
                "已有云端对象校验失败"
            );
        } else {
            Self::status(response)?;
        }
        Ok(())
    }
    fn publish(&self, manifest: &Manifest, revision: Option<&str>) -> Result<()> {
        validate_manifest(manifest)?;
        let bytes = serde_json::to_vec(manifest)?;
        ensure!(
            bytes.len() as u64 <= MAX_MANIFEST_BYTES,
            "同步清单超过大小限制"
        );
        let request = self
            .request(Method::PUT, "inkstone-v1/manifest.json")
            .header(header::CONTENT_TYPE, "application/json")
            .body(bytes);
        let request = match revision {
            Some(etag) => request.header(header::IF_MATCH, etag),
            None => request.header(header::IF_NONE_MATCH, "*"),
        };
        Self::status(self.send(request)?)?;
        Ok(())
    }
}
