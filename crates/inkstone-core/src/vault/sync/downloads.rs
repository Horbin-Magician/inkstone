//! Verified, restart-reusable object staging outside the note library.
use super::*;

pub(super) struct Cache {
    directory: PathBuf,
}
#[derive(Clone)]
pub(super) struct Object {
    path: PathBuf,
    pub bytes: u64,
    digest: String,
}
impl Cache {
    pub fn new(state: &Path) -> Result<Self> {
        let parent = state.parent().context("无效同步状态路径")?;
        ensure!(
            !is_reparse(&fs::symlink_metadata(parent)?),
            "同步暂存目录不能是链接"
        );
        let directory = state.with_extension("downloads");
        fs::create_dir_all(&directory)?;
        let meta = fs::symlink_metadata(&directory)?;
        ensure!(meta.is_dir() && !is_reparse(&meta), "同步暂存目录无效");
        Ok(Self { directory })
    }
    /// Call only while holding the vault sync lock, before starting workers.
    /// Partial objects have never been verified or applied and are not reusable.
    pub fn discard_partial(&self, cancellation: &Cancellation) -> Result<()> {
        for entry in fs::read_dir(&self.directory)? {
            cancellation.check()?;
            let entry = entry?;
            let name = entry.file_name();
            let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".partial")) else {
                continue;
            };
            let fields: Vec<_> = stem.split('-').collect();
            if fields.len() != 3
                || fields
                    .iter()
                    .any(|field| field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()))
            {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_file() && !is_reparse(&metadata) {
                fs::remove_file(entry.path()).context("无法清理未完成的同步下载")?;
            }
        }
        Ok(())
    }
    pub fn fetch(
        &self,
        remote: &impl Remote,
        digest: &str,
        cancellation: &Cancellation,
    ) -> Result<Object> {
        ensure!(valid_hash(digest), "无效的下载对象校验值");
        cancellation.check()?;
        let path = self.directory.join(digest);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                ensure!(meta.is_file() && !is_reparse(&meta), "下载缓存对象类型无效");
                if let Ok(bytes) = verify(&path, digest, cancellation) {
                    return Ok(Object {
                        path,
                        bytes,
                        digest: digest.into(),
                    });
                }
                cancellation.check()?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let temp = self.directory.join(format!("{}.partial", unique_id()));
        let result = (|| -> Result<Object> {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            let mut sink = Sink {
                file,
                digest: Sha256::new(),
                bytes: 0,
                cancellation,
            };
            let received = remote.download_to(digest, &mut sink);
            cancellation.check()?;
            ensure!(received? == sink.bytes, "下载字节数不匹配");
            ensure!(
                format!("{:x}", sink.digest.finalize()) == digest,
                "云端文件校验失败：{digest}"
            );
            sink.file.sync_all()?;
            let bytes = sink.bytes;
            drop(sink.file);
            fs::rename(&temp, &path)?;
            Ok(Object {
                path,
                bytes,
                digest: digest.into(),
            })
        })();
        let _ = fs::remove_file(temp);
        result
    }
}
struct Sink<'a> {
    file: fs::File,
    digest: Sha256,
    bytes: u64,
    cancellation: &'a Cancellation,
}
impl Write for Sink<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancellation.is_requested() {
            return Err(io::Error::other("download cancelled"));
        }
        if bytes.len() as u64 > MAX_FILE_BYTES.saturating_sub(self.bytes) {
            return Err(io::Error::other("云端文件超过大小限制"));
        }
        let count = self.file.write(bytes)?;
        self.digest.update(&bytes[..count]);
        self.bytes += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
fn verify(path: &Path, digest: &str, cancellation: &Cancellation) -> Result<u64> {
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    loop {
        cancellation.check()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        ensure!(bytes <= MAX_FILE_BYTES, "下载缓存对象过大");
        hash.update(&buffer[..count]);
    }
    ensure!(
        format!("{:x}", hash.finalize()) == digest,
        "下载缓存校验失败"
    );
    Ok(bytes)
}
impl Object {
    // The caller only publishes its temporary output after this check succeeds.
    pub fn copy_to(&self, output: &mut impl Write) -> Result<()> {
        let meta = fs::symlink_metadata(&self.path)?;
        ensure!(
            meta.is_file()
                && !is_reparse(&meta)
                && meta.len() == self.bytes
                && self.bytes <= MAX_FILE_BYTES,
            "下载缓存已变化"
        );
        let mut file = fs::File::open(&self.path)?;
        let mut buffer = [0u8; 64 * 1024];
        let mut digest = Sha256::new();
        let mut bytes = 0u64;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            ensure!(bytes <= self.bytes, "下载缓存已变化");
            digest.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        ensure!(
            bytes == self.bytes && format!("{:x}", digest.finalize()) == self.digest,
            "下载缓存校验失败"
        );
        Ok(())
    }
    pub fn remove(&self) {
        let _ = fs::remove_file(&self.path);
    }
}
