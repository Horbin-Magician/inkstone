//! Conservative preflight for cloud cleanup; no object deletion is exposed here.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudCleanupPreview {
    pub candidates: usize,
    pub candidate_bytes: u64,
    pub retained: usize,
    pub retained_bytes: u64,
    identity: String,
    manifest: Manifest,
    revision: Option<String>,
    objects: BTreeMap<String, u64>,
}

/// Keeps the remote maintenance lock alive until explicitly released or dropped.
/// A checked preview is not a durable deletion authorization; any later execution
/// must revalidate the server lock and the fenced manifest before mutating objects.
#[must_use]
pub struct CheckedCloudCleanup<'a> {
    preview: CloudCleanupPreview,
    lock: MaintenanceLock<'a>,
}
impl CheckedCloudCleanup<'_> {
    pub fn preview(&self) -> &CloudCleanupPreview {
        &self.preview
    }
    pub fn release(self) -> Result<()> {
        self.lock.release()
    }
}

#[derive(Debug)]
struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("云端清理已停止；已推进的维护代次不会回退，请重新预览")
    }
}
impl std::error::Error for Cancelled {}
fn check(cancellation: &Cancellation) -> Result<()> {
    if cancellation.is_requested() {
        return Err(Cancelled.into());
    }
    Ok(())
}

impl WebDav {
    /// Read-only candidate estimate. A matching snapshot and a verified,
    /// server-enforced maintenance lock are still required before cleanup.
    pub fn preview_cloud_cleanup(&self) -> Result<CloudCleanupPreview> {
        let (manifest, revision) = self.manifest()?;
        let listing = self.object_listing()?;
        let preview = preview(self.identity(), manifest, revision, listing)?;
        let (after, revision) = self.manifest()?;
        ensure!(
            preview.manifest == after && preview.revision == revision,
            "预览期间云端清单发生变化，请重新预览"
        );
        Ok(preview)
    }

    /// Background-only. Recheck the exact preview under the remote lock, verify
    /// every retained/candidate body, then fence prior writers. This may upgrade
    /// the remote manifest to version 2; it never deletes objects. The caller must
    /// obtain the user's confirmation of that upgrade before invoking this API.
    pub fn prepare_cloud_cleanup(
        &self,
        expected: &CloudCleanupPreview,
        cancellation: &Cancellation,
    ) -> Result<CheckedCloudCleanup<'_>> {
        check(cancellation)?;
        ensure!(
            expected.identity == self.identity(),
            "云端清理预览属于其他服务器或账号"
        );
        ensure!(expected.candidates > 0, "没有可清理的云端旧对象");
        let mut lock = self.lock_maintenance()?;
        check(cancellation)?;
        ensure!(
            self.preview_cloud_cleanup()? == *expected,
            "云端对象或清单已变化，请重新预览"
        );
        for (digest, length) in &expected.objects {
            check(cancellation)?;
            lock.refresh()?;
            let mut verifier = Verifier {
                digest: Sha256::new(),
                cancellation,
            };
            let result = self.download_to(digest, &mut verifier);
            check(cancellation)?;
            let bytes = result?;
            ensure!(
                bytes == *length && format!("{:x}", verifier.digest.finalize()) == *digest,
                "云端对象内容或大小校验失败，已保留全部对象"
            );
        }
        check(cancellation)?;
        lock.refresh()?;
        let (manifest, revision) = lock.advance_manifest()?;
        check(cancellation)?;
        let fenced = CloudCleanupPreview {
            manifest,
            revision: Some(revision),
            ..expected.clone()
        };
        ensure!(
            self.preview_cloud_cleanup()? == fenced,
            "维护期间云端对象或清单已变化，已停止清理"
        );
        check(cancellation)?;
        lock.refresh()?;
        Ok(CheckedCloudCleanup {
            preview: fenced,
            lock,
        })
    }
}

struct Verifier<'a> {
    digest: Sha256,
    cancellation: &'a Cancellation,
}
impl Write for Verifier<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancellation.is_requested() {
            return Err(io::Error::other(Cancelled));
        }
        self.digest.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn preview(
    identity: String,
    manifest: Manifest,
    revision: Option<String>,
    listing: capacity::Listing,
) -> Result<CloudCleanupPreview> {
    ensure!(
        listing.unrecognized == 0,
        "云端对象列表不完整或含未知记录，不能清理"
    );
    let referenced: BTreeSet<_> = manifest.files.values().collect();
    ensure!(
        referenced
            .iter()
            .all(|digest| listing.objects.contains_key(*digest)),
        "云端清单引用的对象缺失，不能清理"
    );
    let mut result = CloudCleanupPreview {
        candidates: 0,
        candidate_bytes: 0,
        retained: 0,
        retained_bytes: 0,
        identity,
        manifest: manifest.clone(),
        revision,
        objects: listing.objects,
    };
    for (digest, length) in &result.objects {
        ensure!(
            *length <= MAX_FILE_BYTES,
            "云端对象超过支持的大小，不能清理"
        );
        let bytes = if referenced.contains(digest) {
            result.retained += 1;
            &mut result.retained_bytes
        } else {
            result.candidates += 1;
            &mut result.candidate_bytes
        };
        *bytes = bytes
            .checked_add(*length)
            .context("云端容量超出可统计范围")?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
