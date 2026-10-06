//! Server-enforced maintenance lease. This alone does not authorize garbage
//! collection: manifest fencing and protected-object validation are also required.
use super::*;
use reqwest::header::HeaderValue;

const PATH: &str = "inkstone/";
const MAX_REPLY: u64 = 64 * 1024;
const SECONDS: u32 = 300;
const LOCK_INFO: &str = r#"<?xml version="1.0"?><d:lockinfo xmlns:d="DAV:"><d:lockscope><d:exclusive/></d:lockscope><d:locktype><d:write/></d:locktype><d:owner>Inkstone maintenance</d:owner></d:lockinfo>"#;

/// Blocking API: acquire, refresh and release only on a background worker.
/// No token is exposed to callers or included in diagnostics. Dropping the guard
/// attempts a bounded release; a failed release must rely on server expiry.
/// A lease is never evidence that a later mutation is authorized: that mutation
/// must submit the lock condition for the server to evaluate atomically.
#[must_use]
pub struct MaintenanceLock<'a> {
    remote: &'a WebDav,
    token: HeaderValue,
    valid: bool,
    released: bool,
    seconds: u32,
}

impl WebDav {
    /// Exclusively lock the existing sync collection and all descendants.
    /// Does not create the collection or mutate a manifest/content object.
    /// Servers returning an infinite, shared, shallow or ambiguous lock are rejected.
    pub fn lock_maintenance(&self) -> Result<MaintenanceLock<'_>> {
        let response = self.send(
            self.request(Method::from_bytes(b"LOCK")?, PATH)
                .header(header::IF_MATCH, "*")
                .header("Depth", "infinity")
                .header("Timeout", format!("Second-{SECONDS}"))
                .header(header::CACHE_CONTROL, "no-cache")
                .header(header::CONTENT_TYPE, "application/xml")
                .body(LOCK_INFO),
        )?;
        ensure!(
            response.status() == StatusCode::OK,
            "服务器未授予云端维护锁（HTTP {}）",
            response.status().as_u16()
        );
        let mut tokens = response.headers().get_all("Lock-Token").iter();
        let token = tokens.next().context("维护锁响应缺少令牌")?.clone();
        ensure!(tokens.next().is_none(), "维护锁响应含多个令牌");
        token_uri(&token)?;
        // A valid returned token lets us release even if body validation fails.
        let mut guard = MaintenanceLock {
            remote: self,
            token,
            valid: false,
            released: false,
            seconds: 0,
        };
        guard.seconds = validate_reply(
            &self.root.join(PATH)?,
            &guard.token,
            &Self::body(response, MAX_REPLY)?,
        )?;
        guard.valid = true;
        Ok(guard)
    }
}

impl MaintenanceLock<'_> {
    /// Server-reported lease duration; scheduling hint only, never authorization.
    pub fn seconds(&self) -> u32 {
        self.seconds
    }

    pub(super) fn condition(&self) -> Result<HeaderValue> {
        ensure!(self.valid && !self.released, "云端维护锁已失效，请重新开始");
        // Tag the lock root, so descendant mutations also test the root's state.
        HeaderValue::from_str(&format!(
            "<{}> ({})",
            self.remote.root.join(PATH)?,
            self.token.to_str().context("维护锁令牌无效")?
        ))
        .context("维护锁条件无效")
    }

    /// An uncertain/failed refresh permanently invalidates this guard for use.
    /// A caller must stop work and acquire a new lock, then revalidate its plan.
    pub fn refresh(&mut self) -> Result<()> {
        let condition = self.condition()?;
        self.valid = false;
        let response = self.remote.send(
            self.remote
                .request(Method::from_bytes(b"LOCK")?, PATH)
                .header("If", condition)
                .header("Timeout", format!("Second-{SECONDS}"))
                .header(header::CACHE_CONTROL, "no-cache"),
        )?;
        ensure!(
            response.status() == StatusCode::OK,
            "云端维护锁续期失败（HTTP {}），已停止维护",
            response.status().as_u16()
        );
        self.seconds = validate_reply(
            &self.remote.root.join(PATH)?,
            &self.token,
            &WebDav::body(response, MAX_REPLY)?,
        )?;
        self.valid = true;
        Ok(())
    }

    /// Irreversibly upgrade to version 2 and advance its maintenance generation.
    /// Call only after the user has elected maintenance requiring all devices to
    /// support version 2. Ordinary sync never invokes this operation.
    ///
    /// A successful conditional publication invalidates pre-maintenance writers.
    /// Read-back must match exactly and expose a different strong ETag. Any failure
    /// invalidates this guard; publication may have succeeded and must not be rolled
    /// back. Reacquire and inspect the manifest before retrying maintenance.
    pub fn advance_manifest(&mut self) -> Result<(Manifest, String)> {
        let result = (|| {
            let condition = self.condition()?;
            let (before, revision) = self.remote.manifest()?;
            let generation = before
                .generation
                .unwrap_or(0)
                .checked_add(1)
                .context("云端维护代次已耗尽，无法继续维护")?;
            let next = Manifest {
                version: 2,
                generation: Some(generation),
                ..before
            };
            self.remote
                .publish_conditionally(&next, revision.as_deref(), Some(condition))?;
            let (confirmed, current_revision) = self.remote.manifest()?;
            let current_revision = current_revision.context("维护后云端清单缺失")?;
            ensure!(
                confirmed == next && revision.as_deref() != Some(current_revision.as_str()),
                "云端维护清单未确认或 ETag 未变化，已停止维护"
            );
            Ok((confirmed, current_revision))
        })();
        if result.is_err() {
            self.valid = false;
        }
        result
    }

    /// Explicit release reports errors. Do this before reporting maintenance success.
    pub fn release(mut self) -> Result<()> {
        self.unlock()
    }

    fn unlock(&mut self) -> Result<()> {
        self.released = true;
        self.valid = false;
        let response = self.remote.send(
            self.remote
                .request(Method::from_bytes(b"UNLOCK")?, PATH)
                .timeout(Duration::from_secs(5))
                .header("Lock-Token", self.token.clone())
                .header(header::CACHE_CONTROL, "no-cache"),
        )?;
        ensure!(
            response.status() == StatusCode::NO_CONTENT,
            "云端维护锁释放未确认（HTTP {}），请等待服务端过期后重试",
            response.status().as_u16()
        );
        Ok(())
    }
}

impl Drop for MaintenanceLock<'_> {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.unlock();
        }
    }
}

fn token_uri(token: &HeaderValue) -> Result<&str> {
    let raw = token.to_str().context("维护锁令牌格式无效")?;
    let uri = raw
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .context("维护锁令牌格式无效")?;
    ensure!(
        !uri.is_empty()
            && uri
                .bytes()
                .all(|b| b.is_ascii_graphic() && !b"<>\"()[]".contains(&b)),
        "维护锁令牌格式无效"
    );
    let parsed = Url::parse(uri).map_err(|_| anyhow::anyhow!("维护锁令牌不是绝对 URI"))?;
    ensure!(parsed.scheme() != "dav", "维护锁令牌使用了保留 URI");
    Ok(uri)
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Result<roxmltree::Node<'a, 'i>> {
    let mut children = node.children().filter(|n| n.has_tag_name(("DAV:", name)));
    let value = children.next().context("维护锁响应缺少必要属性")?;
    ensure!(children.next().is_none(), "维护锁响应存在重复属性");
    Ok(value)
}

fn only_kind(node: roxmltree::Node<'_, '_>, name: &str) -> bool {
    let mut children = node.children().filter(|n| n.is_element());
    children
        .next()
        .is_some_and(|n| n.has_tag_name(("DAV:", name)))
        && children.next().is_none()
}

fn validate_reply(root_url: &Url, token: &HeaderValue, bytes: &[u8]) -> Result<u32> {
    let text = std::str::from_utf8(bytes).context("维护锁响应不是 UTF-8")?;
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 2048,
        },
    )
    .context("维护锁响应 XML 无效")?;
    let root = document.root_element();
    ensure!(root.has_tag_name(("DAV:", "prop")), "维护锁响应缺少 prop");
    let lock = child(child(root, "lockdiscovery")?, "activelock")?;
    ensure!(
        only_kind(child(lock, "lockscope")?, "exclusive")
            && only_kind(child(lock, "locktype")?, "write"),
        "服务器未授予独占写锁"
    );
    ensure!(
        child(lock, "depth")?.text().map(str::trim) == Some("infinity"),
        "服务器未锁定完整同步目录"
    );
    ensure!(
        child(child(lock, "locktoken")?, "href")?
            .text()
            .map(str::trim)
            == Some(token_uri(token)?),
        "维护锁正文与响应令牌不一致"
    );
    let href = child(child(lock, "lockroot")?, "href")?;
    let location = root_url
        .join(href.text().unwrap_or("").trim())
        .context("维护锁目录无效")?;
    ensure!(
        location == *root_url && href.text().is_some_and(|s| !s.trim().is_empty()),
        "维护锁目录与请求不一致"
    );
    let timeout = child(lock, "timeout")?;
    let seconds = timeout
        .text()
        .and_then(|s| s.trim().strip_prefix("Second-"))
        .and_then(|s| s.parse::<u32>().ok())
        .context("服务器未提供有限的维护锁期限")?;
    ensure!(
        (1..=SECONDS).contains(&seconds),
        "服务器维护锁期限超出允许范围"
    );
    Ok(seconds)
}

#[cfg(test)]
mod tests;
