//! Disposable startup cache. File contents remain the source of truth.
use super::{Index, IndexedNote, ParsedNote};
use crate::vault::{Vault, VaultError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

// Bump when parsing semantics or the serialized representation change.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    root: PathBuf,
    notes: BTreeMap<PathBuf, CachedNote>,
}

#[derive(Serialize, Deserialize)]
struct CachedNote {
    digest: [u8; 32],
    parsed: ParsedNote,
}

fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

impl Index {
    /// Cache files live in application data, outside the user's vault.
    pub fn cache_path(vault: &Vault, directory: &Path) -> PathBuf {
        let key = Sha256::digest(vault.root.as_os_str().as_encoded_bytes());
        directory.join(format!("{key:x}.json"))
    }

    pub fn build_cached(vault: &Vault, path: &Path) -> Result<Self, VaultError> {
        crate::startup_trace::mark("cache_read_started");
        let mut cached = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Cache>(&bytes).ok())
            .filter(|cache| cache.version == VERSION && cache.root == vault.root)
            .map(|cache| cache.notes)
            .unwrap_or_default();
        crate::startup_trace::mark("cache_read_done");
        let mut index = Self {
            files: vault.scan_files()?,
            ..Default::default()
        };
        crate::startup_trace::mark("file_scan_done");
        let paths: Vec<_> = index
            .files
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
            .cloned()
            .collect();
        let mut hits = 0usize;
        let mut misses = 0usize;
        for path in paths {
            match vault.read(&path) {
                Ok(Some(text)) => {
                    // Check content, not just size/mtime: sync tools can preserve timestamps.
                    let parsed = cached
                        .remove(&path)
                        .filter(|note| note.digest == digest(&text))
                        .map(|note| {
                            hits += 1;
                            note.parsed
                        })
                        .unwrap_or_else(|| {
                            misses += 1;
                            super::parse(&text)
                        });
                    index.notes.insert(
                        path.clone(),
                        Arc::new(IndexedNote {
                            text,
                            parsed,
                            times: Default::default(),
                        }),
                    );
                    index.refresh_file_times(vault, &path);
                }
                Ok(None) => (),
                Err(error) => {
                    index.errors.insert(path, error.to_string());
                }
            }
        }
        if crate::startup_trace::enabled() {
            crate::startup_trace::mark(&format!("notes_ready hits={hits} misses={misses}"));
        }
        Ok(index)
    }

    /// Best effort; callers run this after presenting the loaded vault.
    pub fn save_cache(&self, vault: &Vault, path: &Path) -> std::io::Result<()> {
        let cache = Cache {
            version: VERSION,
            root: vault.root.clone(),
            notes: self
                .notes
                .iter()
                .map(|(path, note)| {
                    (
                        path.clone(),
                        CachedNote {
                            digest: digest(&note.text),
                            parsed: note.parsed.clone(),
                        },
                    )
                })
                .collect(),
        };
        let bytes = serde_json::to_vec(&cache)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // A partial write is harmless: the next load falls back to a fresh build.
        std::fs::write(path, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox {
        root: PathBuf,
        vault: Vault,
        cache: PathBuf,
    }
    impl Sandbox {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "inkstone-cache-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("vault")).unwrap();
            let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
            let cache = Index::cache_path(&vault, &root.join("cache"));
            Self { root, vault, cache }
        }
        fn write(&self, path: &str, text: &str) {
            std::fs::write(self.vault.root.join(path), text).unwrap();
        }
        fn cached(&self) -> Index {
            Index::build_cached(&self.vault, &self.cache).unwrap()
        }
        fn assert_fresh(&self) {
            let cached = self.cached();
            let fresh = Index::build(&self.vault).unwrap();
            assert_eq!(cached.files, fresh.files);
            assert_eq!(cached.errors, fresh.errors);
            assert_eq!(cached.note_paths(), fresh.note_paths());
            for (path, note) in fresh.notes {
                let restored = &cached.notes[&path];
                assert_eq!(restored.text, note.text);
                assert_eq!(restored.times, note.times);
                assert_eq!(
                    serde_json::to_value(&restored.parsed).unwrap(),
                    serde_json::to_value(&note.parsed).unwrap()
                );
            }
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn cache_matches_fresh_after_edits_renames_deletes_and_read_errors() {
        let s = Sandbox::new();
        s.write("a.md", "---\ntags: [test]\naliases: [alias]\n---\n# Title\n[[b]] #tag\n- [ ] Task\n\n%% comment %%\n^[footnote]\n");
        s.write("b.md", "# Before\n");
        s.assert_fresh();
        s.cached().save_cache(&s.vault, &s.cache).unwrap();
        s.assert_fresh();
        // Same length and timestamp: metadata alone must not validate the cache.
        let path = s.vault.root.join("b.md");
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        s.write("b.md", "# After!\n");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        s.assert_fresh();
        std::fs::rename(s.vault.root.join("a.md"), s.vault.root.join("renamed.md")).unwrap();
        std::fs::remove_file(path).unwrap();
        s.write("new.md", "[[renamed]]\n");
        std::fs::write(s.vault.root.join("invalid.md"), [0xff]).unwrap();
        s.assert_fresh();
    }

    #[test]
    fn invalid_cache_falls_back_and_vaults_are_isolated() {
        let s = Sandbox::new();
        s.write("note.md", "# Actual\n");
        let index = s.cached();
        index.save_cache(&s.vault, &s.cache).unwrap();
        let original = std::fs::read(&s.cache).unwrap();
        for wrong_version in [true, false] {
            let mut cache: Cache = serde_json::from_slice(&original).unwrap();
            if wrong_version {
                cache.version += 1;
            } else {
                cache.root = s.root.join("other");
            }
            cache
                .notes
                .get_mut(Path::new("note.md"))
                .unwrap()
                .parsed
                .headings
                .clear();
            std::fs::write(&s.cache, serde_json::to_vec(&cache).unwrap()).unwrap();
            s.assert_fresh();
        }
        std::fs::write(&s.cache, b"truncated {").unwrap();
        s.assert_fresh();
        let other = Sandbox::new();
        assert_ne!(
            Index::cache_path(&s.vault, Path::new("cache")),
            Index::cache_path(&other.vault, Path::new("cache"))
        );
    }

    #[test]
    #[ignore = "manual startup index timing"]
    fn startup_cache_timing() {
        let s = Sandbox::new();
        let body = "## Heading\n\nParagraph with **bold**, #tag and [[other]].\n\n- [ ] Task\n\n"
            .repeat(30);
        for i in 0..300 {
            s.write(&format!("note-{i}.md"), &body);
        }
        let start = std::time::Instant::now();
        let fresh = Index::build(&s.vault).unwrap();
        let cold = start.elapsed();
        fresh.save_cache(&s.vault, &s.cache).unwrap();
        let start = std::time::Instant::now();
        let cached = s.cached();
        let warm = start.elapsed();
        assert_eq!(cached.note_paths(), fresh.note_paths());
        eprintln!("300 notes: fresh={cold:?}, cached={warm:?}");
    }
}
