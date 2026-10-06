use super::*;

#[derive(Default)]
struct Memory {
    manifest: Mutex<Manifest>,
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
    revision: AtomicUsize,
    fail: AtomicBool,
    downloads: AtomicUsize,
    uploads: AtomicUsize,
    fail_upload: AtomicBool,
}
impl Remote for Memory {
    fn manifest(&self) -> Result<(Manifest, Option<String>)> {
        Ok((
            self.manifest.lock().unwrap().clone(),
            (self.revision.load(Ordering::Relaxed) > 0)
                .then(|| self.revision.load(Ordering::Relaxed).to_string()),
        ))
    }
    fn download(&self, h: &str) -> Result<Vec<u8>> {
        self.downloads.fetch_add(1, Ordering::Relaxed);
        self.objects
            .lock()
            .unwrap()
            .get(h)
            .cloned()
            .context("missing object")
    }
    fn upload(&self, h: &str, b: &[u8]) -> Result<()> {
        ensure!(!self.fail_upload.load(Ordering::Relaxed), "upload failed");
        self.uploads.fetch_add(1, Ordering::Relaxed);
        self.objects.lock().unwrap().insert(h.into(), b.into());
        Ok(())
    }
    fn publish(&self, m: &Manifest, revision: Option<&str>) -> Result<()> {
        ensure!(!self.fail.load(Ordering::Relaxed), "network failure");
        ensure!(
            revision.map(str::to_owned) == self.manifest()?.1,
            "revision conflict"
        );
        *self.manifest.lock().unwrap() = m.clone();
        self.revision.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    a: Vault,
    b: Vault,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("inkstone-sync-{}", unique_id()));
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        Self {
            a: Vault::open(root.join("a"), root.join("recovery")).unwrap(),
            b: Vault::open(root.join("b"), root.join("recovery")).unwrap(),
            root,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn two_devices_sync_notes_binary_edits_deletions_and_renames() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    fs::create_dir_all(f.a.root.join("中文 目录"))?;
    fs::write(f.a.root.join("中文 目录/笔记 #%.md"), "初稿")?;
    fs::write(f.a.root.join("image.png"), [0, 255, 128, 10])?;
    fs::write(f.a.root.join(".inkstone-workspace.json"), "secret")?;
    assert_eq!(synchronize(&f.a, &r, "server")?.uploaded, 2);
    assert_eq!(synchronize(&f.b, &r, "server")?.downloaded, 2);
    assert_eq!(fs::read(f.b.root.join("image.png"))?, [0, 255, 128, 10]);
    fs::write(f.b.root.join("中文 目录/笔记 #%.md"), "第二稿")?;
    synchronize(&f.b, &r, "server")?;
    synchronize(&f.a, &r, "server")?;
    assert_eq!(
        fs::read_to_string(f.a.root.join("中文 目录/笔记 #%.md"))?,
        "第二稿"
    );
    fs::rename(f.a.root.join("image.png"), f.a.root.join("renamed.png"))?;
    synchronize(&f.a, &r, "server")?;
    let report = synchronize(&f.b, &r, "server")?;
    assert_eq!((report.deleted, report.downloaded), (1, 1));
    assert!(!f.b.root.join("image.png").exists());
    assert_eq!(synchronize(&f.b, &r, "server")?.downloaded, 0);
    assert!(
        !r.manifest
            .lock()
            .unwrap()
            .files
            .contains_key(".inkstone-workspace.json")
    );
    Ok(())
}
#[test]
fn conflicts_keep_both_versions_and_converge() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    fs::write(f.a.root.join("note.md"), "base")?;
    synchronize(&f.a, &r, "x")?;
    synchronize(&f.b, &r, "x")?;
    fs::write(f.a.root.join("note.md"), "a")?;
    fs::write(f.b.root.join("note.md"), "b")?;
    synchronize(&f.a, &r, "x")?;
    assert_eq!(synchronize(&f.b, &r, "x")?.conflicts, ["note.md"]);
    synchronize(&f.a, &r, "x")?;
    assert_eq!(snapshot(&f.a)?, snapshot(&f.b)?);
    assert_eq!(snapshot(&f.a)?.len(), 2);
    assert_eq!(fs::read_to_string(f.b.root.join("note.md"))?, "b");
    assert!(synchronize(&f.b, &r, "x")?.conflicts.is_empty());
    Ok(())
}
#[test]
fn edits_win_over_concurrent_deletion() -> Result<()> {
    for delete_remote in [true, false] {
        let f = Fixture::new();
        let r = Memory::default();
        fs::write(f.a.root.join("n.md"), "base")?;
        synchronize(&f.a, &r, "x")?;
        synchronize(&f.b, &r, "x")?;
        let (deleted, edited) = if delete_remote {
            (&f.a, &f.b)
        } else {
            (&f.b, &f.a)
        };
        fs::remove_file(deleted.root.join("n.md"))?;
        fs::write(edited.root.join("n.md"), "edited")?;
        synchronize(&f.a, &r, "x")?;
        assert_eq!(synchronize(&f.b, &r, "x")?.conflicts.len(), 1);
        synchronize(&f.a, &r, "x")?;
        assert_eq!(fs::read_to_string(f.a.root.join("n.md"))?, "edited");
        assert_eq!(snapshot(&f.a)?, snapshot(&f.b)?);
    }
    Ok(())
}
#[test]
fn failed_publish_and_corrupt_download_do_not_modify_local_files() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    fs::write(f.a.root.join("a.md"), "remote")?;
    synchronize(&f.a, &r, "x")?;
    fs::write(f.b.root.join("b.md"), "local")?;
    r.fail.store(true, Ordering::Relaxed);
    assert!(synchronize(&f.b, &r, "x").is_err());
    assert!(!f.b.root.join("a.md").exists());
    r.fail.store(false, Ordering::Relaxed);
    // A damaged staged object must be fetched and verified again.
    let cached = baseline_path(&f.b, "x")
        .with_extension("downloads")
        .join(hash(b"remote"));
    fs::write(cached, "damaged cache")?;
    r.objects
        .lock()
        .unwrap()
        .insert(hash(b"remote"), b"corrupt".to_vec());
    assert!(synchronize(&f.b, &r, "x").is_err());
    assert!(!f.b.root.join("a.md").exists());
    r.objects
        .lock()
        .unwrap()
        .insert(hash(b"remote"), b"remote".to_vec());
    synchronize(&f.b, &r, "x")?;
    assert_eq!(snapshot(&f.b)?.len(), 2);
    Ok(())
}
#[test]
fn rejects_unsafe_paths_aliases_missing_manifest_and_endpoint_confusion() -> Result<()> {
    for path in [
        "../secret",
        "/root",
        "a//b",
        "a\\b",
        "C:/file",
        ".git/config",
        "a/.inkstone-trash/x",
        "a.",
        "CON.md",
    ] {
        assert!(validate_path(path).is_err(), "{path}");
    }
    for names in [["a.md", "A.md"], ["a", "a/b.md"]] {
        let m = Manifest {
            version: 1,
            files: names.into_iter().map(|p| (p.into(), hash(b"x"))).collect(),
        };
        assert!(validate_manifest(&m).is_err());
    }
    let f = Fixture::new();
    let r = Memory::default();
    fs::write(f.a.root.join("a.md"), "x")?;
    synchronize(&f.a, &r, "one")?;
    let other = Memory::default();
    assert!(synchronize(&f.a, &other, "one").is_err());
    assert!(synchronize(&f.a, &other, "two").is_ok());
    Ok(())
}
#[cfg(unix)]
#[test]
fn rejects_symlink_download_targets() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    fs::write(f.a.root.join("n.md"), "remote")?;
    synchronize(&f.a, &r, "x")?;
    let outside = f.root.join("outside");
    fs::write(&outside, "safe")?;
    std::os::unix::fs::symlink(&outside, f.b.root.join("n.md"))?;
    assert!(synchronize(&f.b, &r, "x").is_err());
    assert_eq!(fs::read_to_string(outside)?, "safe");
    Ok(())
}

// Exercise actual HTTP serialization, authentication, preconditions and errors.
fn server(replies: Vec<&'static str>) -> (Settings, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let settings = Settings {
        url: format!("http://{}/dav/", listener.local_addr().unwrap()),
        username: "user".into(),
        auto: true,
        ..Settings::default()
    };
    let handle = std::thread::spawn(move || {
        let mut requests = vec![];
        for reply in replies {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut request = String::new();
            let mut size = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.to_lowercase().starts_with("content-length:") {
                    size = line
                        .split(':')
                        .nth(1)
                        .unwrap()
                        .trim()
                        .parse::<usize>()
                        .unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; size];
            reader.read_exact(&mut body).unwrap();
            request.push_str(&String::from_utf8_lossy(&body));
            requests.push(request);
            socket.write_all(reply.as_bytes()).unwrap();
        }
        requests
    });
    (settings, handle)
}
#[test]
fn webdav_auth_conditional_creation_and_concurrent_write_rejection() -> Result<()> {
    let (settings, thread) = server(vec![
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 412 Precondition Failed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    ]);
    let dav = WebDav::new(&settings, "pass")?;
    assert!(dav.manifest()?.1.is_none());
    dav.publish(&Manifest::default(), None)?;
    assert!(dav.publish(&Manifest::default(), Some("\"old\"")).is_err());
    let requests = thread.join().unwrap();
    assert!(requests[0].starts_with("GET /dav/inkstone-v1/manifest.json"));
    let requests = &requests[1..];
    assert!(requests[0].starts_with("GET /dav/inkstone/manifest.json"));
    assert!(
        requests[0]
            .to_lowercase()
            .contains("authorization: basic dxnlcjpwyxnz")
    );
    assert!(requests[1].to_lowercase().contains("if-none-match: *"));
    assert!(requests[2].to_lowercase().contains("if-match: \"old\""));
    Ok(())
}
#[test]
fn webdav_refuses_redirects_weak_etags_and_invalid_urls() -> Result<()> {
    for reply in [
        "HTTP/1.1 302 Found\r\nLocation: http://localhost/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 200 OK\r\nETag: W/\"weak\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    ] {
        let (settings, thread) = server(vec![
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            reply,
        ]);
        assert!(WebDav::new(&settings, "secret")?.manifest().is_err());
        thread.join().unwrap();
    }
    for url in [
        "file:///tmp",
        "https://user:secret@example.org",
        "https://example.org/?token=secret",
    ] {
        assert!(
            WebDav::new(
                &Settings {
                    url: url.into(),
                    username: String::new(),
                    auto: true,
                    ..Settings::default()
                },
                ""
            )
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn local_replacement_and_deletion_leave_identifiable_recovery_copies() -> Result<()> {
    let f = Fixture::new();
    let path = f.a.root.join("note.md");
    fs::write(&path, "original")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    apply_bytes(
        &f.a,
        "note.md",
        Some(&hash(b"original")),
        Some(b"downloaded"),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
    }
    apply_bytes(&f.a, "note.md", Some(&hash(b"downloaded")), None)?;
    assert!(!path.exists());
    let mut contents = BTreeSet::new();
    for entry in fs::read_dir(&f.a.root)? {
        let entry = entry?;
        if entry.path().extension().is_some_and(|e| e == "json") {
            let metadata: serde_json::Value = serde_json::from_slice(&fs::read(entry.path())?)?;
            assert_eq!(metadata["original"], "note.md");
            let bytes = fs::read(f.a.root.join(metadata["backup"].as_str().unwrap()))?;
            assert_eq!(metadata["sha256"], hash(&bytes));
            contents.insert(bytes);
        }
    }
    assert_eq!(
        contents,
        BTreeSet::from([b"original".to_vec(), b"downloaded".to_vec()])
    );
    assert!(snapshot(&f.a)?.is_empty());
    // Overlong conflict filenames are rejected before remote publication.
    assert!(validate_path(&format!("{}.md", "n".repeat(253))).is_err());
    Ok(())
}

#[test]
fn duplicate_content_transfers_once_and_unchanged_sync_transfers_nothing() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    for i in 0..12 {
        fs::write(f.a.root.join(format!("{i}.md")), "shared content")?;
    }
    assert_eq!(synchronize(&f.a, &r, "x")?.uploaded, 1);
    assert_eq!(synchronize(&f.b, &r, "x")?.downloaded, 12);
    assert_eq!(snapshot(&f.a)?, snapshot(&f.b)?);
    synchronize(&f.a, &r, "x")?;
    synchronize(&f.b, &r, "x")?;
    assert_eq!(r.uploads.load(Ordering::Relaxed), 1);
    assert_eq!(r.downloads.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn transfers_overlap_and_join_before_returning() -> Result<()> {
    let barrier = std::sync::Barrier::new(4);
    let active = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    transfer(&[0, 1, 2, 3], |_| {
        active.fetch_add(1, Ordering::SeqCst);
        barrier.wait();
        assert_eq!(active.load(Ordering::SeqCst), 4);
        barrier.wait();
        completed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })?;
    assert_eq!(completed.load(Ordering::SeqCst), 4);
    Ok(())
}

#[test]
fn failed_transfer_does_not_publish_or_apply_downloads() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    fs::write(f.a.root.join("remote.md"), "remote")?;
    synchronize(&f.a, &r, "x")?;
    let revision = r.revision.load(Ordering::Relaxed);
    for i in 0..10 {
        fs::write(f.b.root.join(format!("{i}.md")), format!("local {i}"))?;
    }
    r.fail_upload.store(true, Ordering::Relaxed);
    assert!(synchronize(&f.b, &r, "x").is_err());
    assert_eq!(r.revision.load(Ordering::Relaxed), revision);
    assert!(!f.b.root.join("remote.md").exists());
    r.fail_upload.store(false, Ordering::Relaxed);
    synchronize(&f.b, &r, "x")?;
    assert_eq!(fs::read_to_string(f.b.root.join("remote.md"))?, "remote");
    Ok(())
}

#[test]
fn progress_tracks_each_phase_and_only_completes_after_success() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    for i in 0..9 {
        fs::write(f.a.root.join(format!("{i}.md")), format!("content {i}"))?;
    }
    let events = Mutex::new(Vec::new());
    synchronize_with_progress(&f.a, &r, "x", |p| events.lock().unwrap().push(p))?;
    let events = events.into_inner().unwrap();
    assert_eq!(events.first().unwrap().phase, Phase::Scanning);
    assert_eq!(events.last().unwrap().phase, Phase::Complete);
    for phase in [Phase::Scanning, Phase::Uploading, Phase::Verifying] {
        let steps: Vec<_> = events
            .iter()
            .filter(|p| p.phase == phase && p.total > 0)
            .collect();
        assert_eq!(steps.first().unwrap().completed, 0);
        assert_eq!(steps.last().unwrap().completed, 9);
        assert_eq!(steps.last().unwrap().bytes, 81);
        for pair in steps.windows(2) {
            assert_eq!(pair[1].completed, pair[0].completed + 1);
            assert!(pair[1].bytes >= pair[0].bytes);
        }
    }
    let events = Mutex::new(Vec::new());
    synchronize_with_progress(&f.b, &r, "x", |p| events.lock().unwrap().push(p))?;
    let events = events.into_inner().unwrap();
    for phase in [Phase::Downloading, Phase::Applying] {
        let last = events.iter().rfind(|p| p.phase == phase).unwrap();
        assert_eq!((last.completed, last.total), (9, 9));
    }
    fs::write(f.b.root.join("new.md"), "new")?;
    r.fail.store(true, Ordering::Relaxed);
    let events = Mutex::new(Vec::new());
    assert!(synchronize_with_progress(&f.b, &r, "x", |p| events.lock().unwrap().push(p)).is_err());
    let events = events.into_inner().unwrap();
    assert_eq!(events.last().unwrap().phase, Phase::Publishing);
    assert!(
        !events
            .iter()
            .any(|p| matches!(p.phase, Phase::Complete | Phase::Applying))
    );
    Ok(())
}

#[test]
fn webdav_legacy_manifest_blocks_preparation_and_sync() -> Result<()> {
    for prepare in [false, true] {
        let mut replies = vec![];
        if prepare {
            replies.push(
                "HTTP/1.1 207 Multi-Status\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
        replies.push("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let (settings, thread) = server(replies);
        let dav = WebDav::new(&settings, "pass")?;
        let error = if prepare {
            dav.prepare().unwrap_err()
        } else {
            dav.manifest().err().unwrap()
        };
        assert!(error.to_string().contains("重命名"));
        let requests = thread.join().unwrap();
        assert!(
            requests
                .last()
                .unwrap()
                .starts_with("GET /dav/inkstone-v1/manifest.json")
        );
        assert!(
            requests
                .iter()
                .all(|r| !r.starts_with("MKCOL") && !r.starts_with("PUT"))
        );
    }
    Ok(())
}

#[test]
fn webdav_uses_unversioned_directory_and_manifest_version() -> Result<()> {
    let (settings, thread) = server(vec![
        "HTTP/1.1 207 Multi-Status\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx",
        "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    ]);
    let dav = WebDav::new(&settings, "pass")?;
    dav.prepare()?;
    let digest = hash(b"x");
    dav.upload(&digest, b"x")?;
    assert_eq!(dav.download(&digest)?, b"x");
    dav.publish(&Manifest::default(), None)?;
    let requests = thread.join().unwrap();
    assert!(requests[2].starts_with("MKCOL /dav/inkstone/ HTTP/"));
    assert!(requests[3].starts_with("MKCOL /dav/inkstone/objects/ HTTP/"));
    assert!(requests[4].starts_with(&format!("PUT /dav/inkstone/objects/{digest} HTTP/")));
    assert!(requests[5].starts_with(&format!("GET /dav/inkstone/objects/{digest} HTTP/")));
    assert!(requests[6].starts_with("PUT /dav/inkstone/manifest.json HTTP/"));
    let body = requests[6].split("\r\n\r\n").nth(1).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body)?["version"],
        1
    );
    assert!(
        validate_manifest(&Manifest {
            version: 2,
            files: Files::new()
        })
        .is_err()
    );
    Ok(())
}

#[test]
fn older_settings_default_to_five_minute_remote_checks() {
    let settings: Settings =
        serde_json::from_str(r#"{"url":"https://example.test/dav/","username":"user"}"#).unwrap();
    assert!(settings.auto);
    assert_eq!(settings.poll_minutes, 5);
    let mut custom = settings;
    custom.poll_minutes = 15;
    let roundtrip: Settings =
        serde_json::from_slice(&serde_json::to_vec(&custom).unwrap()).unwrap();
    assert_eq!(roundtrip.poll_minutes, 15);
}

#[test]
fn watcher_classification_ignores_sync_writes_but_detects_external_changes() {
    let f = Fixture::new();
    let remote = Memory::default();
    let paths = BTreeSet::from([PathBuf::from("note.md"), PathBuf::from("asset.bin")]);
    fs::write(f.a.root.join("note.md"), "first").unwrap();
    fs::write(f.a.root.join("asset.bin"), [0, 1, 255]).unwrap();
    synchronize(&f.a, &remote, "watch").unwrap();
    synchronize(&f.b, &remote, "watch").unwrap();
    assert!(!changed_since_sync(&f.b, "watch", &paths).unwrap());
    fs::write(f.b.root.join("asset.bin"), [0, 2, 255]).unwrap();
    assert!(changed_since_sync(&f.b, "watch", &paths).unwrap());
    synchronize(&f.b, &remote, "watch").unwrap();
    assert!(!changed_since_sync(&f.b, "watch", &paths).unwrap());
    fs::remove_file(f.b.root.join("note.md")).unwrap();
    assert!(changed_since_sync(&f.b, "watch", &paths).unwrap());
    synchronize(&f.b, &remote, "watch").unwrap();
    synchronize(&f.a, &remote, "watch").unwrap();
    assert!(!changed_since_sync(&f.a, "watch", &paths).unwrap());
    assert!(changed_since_sync(&f.a, "other-endpoint", &paths).unwrap());
}

#[test]
fn watcher_classification_reconciles_directories_and_rejects_bad_state() {
    let f = Fixture::new();
    let remote = Memory::default();
    fs::create_dir(f.a.root.join("folder")).unwrap();
    fs::write(f.a.root.join("folder/note.md"), "body").unwrap();
    synchronize(&f.a, &remote, "watch").unwrap();
    let paths = BTreeSet::from([PathBuf::from("folder")]);
    assert!(!changed_since_sync(&f.a, "watch", &paths).unwrap());
    fs::rename(f.a.root.join("folder"), f.a.root.join("renamed")).unwrap();
    assert!(changed_since_sync(&f.a, "watch", &paths).unwrap());
    assert!(changed_since_sync(&f.a, "watch", &BTreeSet::from([PathBuf::new()])).unwrap());
    assert!(
        !changed_since_sync(&f.a, "watch", &BTreeSet::from([PathBuf::from(".hidden")])).unwrap()
    );
    assert!(
        changed_since_sync(&f.a, "watch", &BTreeSet::from([PathBuf::from("../escape")])).is_err()
    );
    fs::write(baseline_path(&f.a, "watch"), "corrupt").unwrap();
    assert!(changed_since_sync(&f.a, "watch", &paths).is_err());
}

#[test]
fn success_time_is_durable_scoped_and_not_advanced_by_failure() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    assert_eq!(last_success(&f.a, "server")?, None);
    let report = synchronize(&f.a, &r, "server")?;
    assert!(report.completed_at_ms > 0);
    assert_eq!(last_success(&f.a, "server")?, Some(report.completed_at_ms));
    assert_eq!(last_success(&f.b, "server")?, None);
    assert_eq!(last_success(&f.a, "other")?, None);
    fs::write(f.a.root.join("new.md"), "changed")?;
    r.fail.store(true, Ordering::Relaxed);
    assert!(synchronize(&f.a, &r, "server").is_err());
    assert_eq!(last_success(&f.a, "server")?, Some(report.completed_at_ms));
    // The previous format remains readable, without inventing a success date.
    fs::write(
        baseline_path(&f.a, "server"),
        serde_json::to_vec(&Manifest::default())?,
    )?;
    assert_eq!(last_success(&f.a, "server")?, None);
    fs::write(baseline_path(&f.a, "server"), "broken")?;
    assert!(last_success(&f.a, "server").is_err());
    Ok(())
}

#[test]
fn cancellation_before_publication_keeps_baseline_and_local_files_and_can_retry() -> Result<()> {
    for phase in [
        Phase::Scanning,
        Phase::ReadingManifest,
        Phase::Downloading,
        Phase::Uploading,
        Phase::Verifying,
        Phase::Publishing,
    ] {
        let f = Fixture::new();
        let r = Memory::default();
        fs::write(f.b.root.join("remote.md"), "remote")?;
        synchronize(&f.b, &r, "cancel")?;
        fs::write(f.a.root.join("local.md"), "local")?;
        let token = Cancellation::default();
        let result = synchronize_cancellable(&f.a, &r, "cancel", &token, |p| {
            if p.phase == phase {
                token.request();
            }
        });
        assert!(is_cancelled(&result.unwrap_err()), "{phase:?}");
        assert_eq!(fs::read_to_string(f.a.root.join("local.md"))?, "local");
        assert!(!f.a.root.join("remote.md").exists());
        assert_eq!(last_success(&f.a, "cancel")?, None);
        assert!(!r.manifest.lock().unwrap().files.contains_key("local.md"));
        synchronize(&f.a, &r, "cancel")?;
        assert_eq!(fs::read_to_string(f.a.root.join("remote.md"))?, "remote");
        assert!(r.manifest.lock().unwrap().files.contains_key("local.md"));
    }
    Ok(())
}

#[test]
fn cancellation_after_commit_begins_is_rejected_and_finishes_baseline() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    fs::write(f.b.root.join("remote.md"), "remote")?;
    synchronize(&f.b, &r, "cancel")?;
    let token = Cancellation::default();
    let requested = AtomicBool::new(false);
    let report = synchronize_cancellable(&f.a, &r, "cancel", &token, |p| {
        if p.phase == Phase::Applying {
            requested.store(true, Ordering::Relaxed);
            assert!(!token.request());
        }
    })?;
    assert!(requested.load(Ordering::Relaxed));
    assert_eq!(fs::read_to_string(f.a.root.join("remote.md"))?, "remote");
    assert_eq!(last_success(&f.a, "cancel")?, Some(report.completed_at_ms));
    Ok(())
}

#[test]
fn webdav_streams_chunked_objects_and_rejects_truncated_responses() -> Result<()> {
    let (settings, thread) = server(vec![
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n4\r\ndefg\r\n0\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nshort",
    ]);
    let dav = WebDav::new(&settings, "pass")?;
    let mut received = Vec::new();
    assert_eq!(dav.download_to(&hash(b"abcdefg"), &mut received)?, 7);
    assert_eq!(received, b"abcdefg");
    assert!(dav.download_to(&hash(b"short"), &mut io::sink()).is_err());
    assert_eq!(thread.join().unwrap().len(), 2);
    Ok(())
}

#[test]
fn failed_publication_reuses_verified_staging_after_reopening_vault() -> Result<()> {
    let f = Fixture::new();
    let r = Memory::default();
    fs::write(f.a.root.join("a.md"), "remote")?;
    synchronize(&f.a, &r, "x")?;
    fs::write(f.b.root.join("b.md"), "local")?;
    r.fail.store(true, Ordering::Relaxed);
    assert!(synchronize(&f.b, &r, "x").is_err());
    let count = r.downloads.load(Ordering::Relaxed);
    assert_eq!(count, 1);
    assert!(!f.b.root.join("a.md").exists());
    r.objects.lock().unwrap().clear(); // Network objects now unavailable.
    r.fail.store(false, Ordering::Relaxed);
    let reopened = Vault::open(f.b.root.clone(), f.root.join("recovery"))?;
    synchronize(&reopened, &r, "x")?;
    assert_eq!(r.downloads.load(Ordering::Relaxed), count);
    assert_eq!(fs::read_to_string(f.b.root.join("a.md"))?, "remote");
    assert_eq!(fs::read_to_string(f.b.root.join("b.md"))?, "local");
    assert_eq!(
        fs::read_dir(baseline_path(&f.b, "x").with_extension("downloads"))?.count(),
        0
    );
    Ok(())
}

#[test]
#[ignore = "writes and verifies 576 MiB; run explicitly for large-library acceptance"]
fn streamed_first_sync_exceeds_former_batch_limit() -> Result<()> {
    struct Generated {
        manifest: Manifest,
        objects: BTreeMap<String, u8>,
    }
    const CHUNKS: usize = 1024;
    impl Remote for Generated {
        fn manifest(&self) -> Result<(Manifest, Option<String>)> {
            Ok((self.manifest.clone(), Some("1".into())))
        }
        fn download(&self, _: &str) -> Result<Vec<u8>> {
            panic!("large sync must use the streaming interface")
        }
        fn download_to(&self, digest: &str, output: &mut dyn Write) -> Result<u64> {
            let chunk = [self.objects[digest]; 64 * 1024];
            for _ in 0..CHUNKS {
                output.write_all(&chunk)?;
            }
            Ok((CHUNKS * chunk.len()) as u64)
        }
        fn upload(&self, _: &str, _: &[u8]) -> Result<()> {
            panic!("unexpected upload")
        }
        fn publish(&self, _: &Manifest, _: Option<&str>) -> Result<()> {
            panic!("unexpected publish")
        }
    }
    let f = Fixture::new();
    let mut r = Generated {
        manifest: Manifest::default(),
        objects: BTreeMap::new(),
    };
    for value in 0..9u8 {
        let mut digest = Sha256::new();
        for _ in 0..CHUNKS {
            digest.update([value; 64 * 1024]);
        }
        let digest = format!("{:x}", digest.finalize());
        r.manifest
            .files
            .insert(format!("{value}.bin"), digest.clone());
        r.objects.insert(digest, value);
    }
    assert_eq!(synchronize(&f.b, &r, "large")?.downloaded, 9);
    assert_eq!(snapshot(&f.b)?, r.manifest.files);
    for value in 0..9u8 {
        assert_eq!(
            fs::metadata(f.b.root.join(format!("{value}.bin")))?.len(),
            64 * 1024 * 1024
        );
    }
    assert_eq!(
        fs::read_dir(baseline_path(&f.b, "large").with_extension("downloads"))?.count(),
        0
    );
    Ok(())
}

#[test]
fn cancelled_stream_removes_partial_and_preserves_verified_objects() -> Result<()> {
    struct Interrupted<'a>(&'a Cancellation);
    impl Remote for Interrupted<'_> {
        fn manifest(&self) -> Result<(Manifest, Option<String>)> {
            unreachable!()
        }
        fn download(&self, _: &str) -> Result<Vec<u8>> {
            unreachable!()
        }
        fn download_to(&self, _: &str, output: &mut dyn Write) -> Result<u64> {
            output.write_all(b"partial")?;
            assert!(self.0.request());
            output.write_all(b"remaining")?;
            unreachable!()
        }
        fn upload(&self, _: &str, _: &[u8]) -> Result<()> {
            unreachable!()
        }
        fn publish(&self, _: &Manifest, _: Option<&str>) -> Result<()> {
            unreachable!()
        }
    }
    let f = Fixture::new();
    let state = baseline_path(&f.b, "cancel");
    fs::create_dir_all(state.parent().unwrap())?;
    let cache = downloads::Cache::new(&state)?;
    let r = Memory::default();
    r.objects
        .lock()
        .unwrap()
        .insert(hash(b"verified"), b"verified".to_vec());
    let object = cache.fetch(&r, &hash(b"verified"), &Cancellation::default())?;
    let cancellation = Cancellation::default();
    let error = cache
        .fetch(
            &Interrupted(&cancellation),
            &hash(b"partialremaining"),
            &cancellation,
        )
        .err()
        .unwrap();
    assert!(is_cancelled(&error));
    let mut copied = Vec::new();
    object.copy_to(&mut copied)?;
    assert_eq!(copied, b"verified");
    assert_eq!(fs::read_dir(state.with_extension("downloads"))?.count(), 1);
    // A changed staged file is rejected before local application.
    fs::write(
        state.with_extension("downloads").join(hash(b"verified")),
        b"modified",
    )?;
    assert!(object.copy_to(&mut Vec::new()).is_err());
    Ok(())
}

#[test]
fn streaming_fingerprint_preserves_digest_limits_and_cancellation() -> Result<()> {
    let f = Fixture::new();
    let path = f.b.root.join("binary.bin");
    let cancellation = Cancellation::default();
    assert!(fingerprint(&path, &cancellation)?.is_none());
    for bytes in [Vec::new(), (0..200_003).map(|n| (n % 251) as u8).collect()] {
        fs::write(&path, &bytes)?;
        assert_eq!(
            fingerprint(&path, &cancellation)?,
            Some((hash(&bytes), bytes.len() as u64))
        );
    }
    assert!(fingerprint(&f.b.root, &cancellation).is_err());
    assert!(cancellation.request());
    assert!(is_cancelled(
        &fingerprint(&path, &cancellation).unwrap_err()
    ));
    fs::File::create(&path)?.set_len(MAX_FILE_BYTES + 1)?;
    assert!(fingerprint(&path, &Cancellation::default()).is_err());
    Ok(())
}

#[test]
fn failed_streamed_application_keeps_original_and_cleans_temporary_file() -> Result<()> {
    let f = Fixture::new();
    let path = f.b.root.join("note.md");
    fs::write(&path, "old")?;
    let error = apply(
        &f.b,
        "note.md",
        Some(&hash(b"old")),
        Some(&|file| {
            file.write_all(b"partial")?;
            bail!("injected destination failure")
        }),
    );
    assert!(error.is_err());
    assert_eq!(fs::read(&path)?, b"old");
    assert!(
        !fs::read_dir(&f.b.root)?.any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "tmp"))
    );

    let r = Memory::default();
    fs::write(f.a.root.join("note.md"), "old")?;
    synchronize(&f.a, &r, "x")?;
    synchronize(&f.b, &r, "x")?;
    let baseline = fs::read(baseline_path(&f.b, "x"))?;
    fs::write(f.a.root.join("note.md"), "new")?;
    synchronize(&f.a, &r, "x")?;
    let result = synchronize_with_progress(&f.b, &r, "x", |progress| {
        if progress.phase == Phase::Applying && progress.completed == 0 {
            // Same length, wrong digest: only the copy's hash can reject this.
            fs::write(
                baseline_path(&f.b, "x")
                    .with_extension("downloads")
                    .join(hash(b"new")),
                b"bad",
            )
            .unwrap();
        }
    });
    assert!(result.is_err());
    assert_eq!(fs::read(&path)?, b"old");
    assert_eq!(fs::read(baseline_path(&f.b, "x"))?, baseline);
    assert!(
        !fs::read_dir(&f.b.root)?.any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "tmp"))
    );
    synchronize(&f.b, &r, "x")?;
    assert_eq!(fs::read(&path)?, b"new");
    Ok(())
}

#[test]
fn killed_download_process_reuses_only_complete_verified_objects() -> Result<()> {
    const CHILD: &str = "INKSTONE_SYNC_DOWNLOAD_CHILD";
    const COMPLETE: &[u8] = b"verified complete object";
    const INCOMPLETE: &[u8] = b"interrupted object must be downloaded again";
    struct RemoteProcess {
        root: PathBuf,
        child: bool,
        downloads: AtomicUsize,
    }
    impl Remote for RemoteProcess {
        fn manifest(&self) -> Result<(Manifest, Option<String>)> {
            Ok((
                Manifest {
                    version: 1,
                    files: [
                        ("complete.md".into(), hash(COMPLETE)),
                        ("interrupted.md".into(), hash(INCOMPLETE)),
                    ]
                    .into(),
                },
                Some("1".into()),
            ))
        }
        fn download(&self, _: &str) -> Result<Vec<u8>> {
            unreachable!()
        }
        fn download_to(&self, digest: &str, output: &mut dyn Write) -> Result<u64> {
            self.downloads.fetch_add(1, Ordering::Relaxed);
            if digest == hash(COMPLETE) {
                ensure!(
                    self.child,
                    "verified object must survive process termination"
                );
                output.write_all(COMPLETE)?;
                return Ok(COMPLETE.len() as u64);
            }
            ensure!(digest == hash(INCOMPLETE), "unexpected object");
            if self.child {
                output.write_all(b"interrupted")?;
                fs::write(self.root.join("partial-ready"), b"ready")?;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
            output.write_all(INCOMPLETE)?;
            Ok(INCOMPLETE.len() as u64)
        }
        fn upload(&self, _: &str, _: &[u8]) -> Result<()> {
            unreachable!()
        }
        fn publish(&self, _: &Manifest, _: Option<&str>) -> Result<()> {
            unreachable!()
        }
    }
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        let vault = Vault::open(root.join("b"), root.join("recovery"))?;
        let remote = RemoteProcess {
            root: root.clone(),
            child: true,
            downloads: AtomicUsize::new(0),
        };
        synchronize_with_progress(&vault, &remote, "crash", |progress| {
            if progress.phase == Phase::Downloading && progress.completed == 1 {
                fs::write(root.join("complete-ready"), b"ready").unwrap();
            }
        })?;
        panic!("child must be killed while downloading");
    }
    let f = Fixture::new();
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "vault::sync::tests::killed_download_process_reuses_only_complete_verified_objects",
        ])
        .env(CHILD, &f.root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let ready = loop {
        if f.root.join("partial-ready").exists() && f.root.join("complete-ready").exists() {
            break true;
        }
        if child.try_wait()?.is_some() || std::time::Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let _ = child.kill();
    let status = child.wait()?;
    assert!(
        ready,
        "child did not reach both download checkpoints: {status}"
    );
    assert!(!status.success());
    assert!(!f.b.root.join("complete.md").exists());
    assert!(!f.b.root.join("interrupted.md").exists());
    let state = baseline_path(&f.b, "crash");
    assert!(!state.exists());
    let cache = state.with_extension("downloads");
    assert_eq!(fs::read(cache.join(hash(COMPLETE)))?, COMPLETE);
    assert!(!cache.join(hash(INCOMPLETE)).exists());
    assert!(fs::read_dir(&cache)?.any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "partial")
    }));
    let reopened = Vault::open(f.b.root.clone(), f.root.join("recovery"))?;
    let remote = RemoteProcess {
        root: f.root.clone(),
        child: false,
        downloads: AtomicUsize::new(0),
    };
    assert_eq!(synchronize(&reopened, &remote, "crash")?.downloaded, 2);
    assert_eq!(remote.downloads.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(f.b.root.join("complete.md"))?, COMPLETE);
    assert_eq!(fs::read(f.b.root.join("interrupted.md"))?, INCOMPLETE);
    assert!(state.exists());
    assert_eq!(fs::read_dir(cache)?.count(), 0);
    Ok(())
}

#[test]
fn abandoned_download_cleanup_is_locked_and_confined_to_owned_partial_files() -> Result<()> {
    let f = Fixture::new();
    let cache = baseline_path(&f.b, "x").with_extension("downloads");
    let other = baseline_path(&f.b, "other").with_extension("downloads");
    let other_vault = baseline_path(&f.a, "x").with_extension("downloads");
    for dir in [&cache, &other, &other_vault] {
        fs::create_dir_all(dir)?;
        fs::write(dir.join("1-2-3.partial"), b"partial")?;
    }
    let digest = hash(b"verified");
    fs::write(cache.join(&digest), b"verified")?;
    fs::write(cache.join("notes.partial"), b"unknown")?;
    fs::write(cache.join("1-2.partial"), b"unknown")?;
    fs::create_dir(cache.join("4-5-6.partial"))?;
    let recovery = f.b.journal(Path::new("note.md"), None, "unsaved draft")?;
    let before = fs::read(&recovery)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&recovery, cache.join("7-8-9.partial"))?;
    let lock_path = f.root.join("recovery/webdav-sync").join(format!(
        "{}.lock",
        hash(f.b.root.to_string_lossy().as_bytes())
    ));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock.try_lock()?;
    assert!(synchronize(&f.b, &Memory::default(), "x").is_err());
    assert!(cache.join("1-2-3.partial").exists());
    drop(lock);
    synchronize(&f.b, &Memory::default(), "x")?;
    assert!(!cache.join("1-2-3.partial").exists());
    assert_eq!(fs::read(cache.join(digest))?, b"verified");
    for path in [
        cache.join("notes.partial"),
        cache.join("1-2.partial"),
        cache.join("4-5-6.partial"),
        other.join("1-2-3.partial"),
        other_vault.join("1-2-3.partial"),
    ] {
        assert!(path.exists());
    }
    assert_eq!(fs::read(recovery)?, before);
    #[cfg(unix)]
    assert!(
        fs::symlink_metadata(cache.join("7-8-9.partial"))?
            .file_type()
            .is_symlink()
    );
    Ok(())
}

#[test]
fn webdav_file_upload_streams_full_snapshot_and_verifies_existing_objects() -> Result<()> {
    use std::io::Seek;
    let f = Fixture::new();
    let path = f.root.join("upload.snapshot");
    let bytes = vec![b'x'; 200_003];
    fs::write(&path, &bytes)?;
    let (settings, thread) = server(vec![
        "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    ]);
    let dav = WebDav::new(&settings, "pass")?;
    let mut file = fs::File::open(&path)?;
    file.seek(std::io::SeekFrom::Start(17))?;
    dav.upload_file(&hash(&bytes), file)?;
    let requests = thread.join().unwrap();
    assert_eq!(requests.len(), 1);
    let (headers, body) = requests[0].split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with(&format!("PUT /dav/inkstone/objects/{} ", hash(&bytes))));
    assert!(headers.to_lowercase().contains("content-length: 200003"));
    assert!(headers.to_lowercase().contains("if-none-match: *"));
    assert_eq!(body.as_bytes(), bytes);
    // Reject wrong digests and oversized files before making a request.
    assert!(
        dav.upload_file(&hash(b"different"), fs::File::open(&path)?)
            .is_err()
    );
    fs::File::create(&path)?.set_len(MAX_FILE_BYTES + 1)?;
    assert!(
        dav.upload_file(&hash(b"x"), fs::File::open(&path)?)
            .is_err()
    );
    fs::write(&path, b"x")?;
    for (reply, expected) in [
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx",
            true,
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\ny",
            false,
        ),
    ] {
        let (settings, thread) = server(vec![
            "HTTP/1.1 412 Precondition Failed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            reply,
        ]);
        assert_eq!(
            WebDav::new(&settings, "pass")?
                .upload_file(&hash(b"x"), fs::File::open(&path)?)
                .is_ok(),
            expected
        );
        let requests = thread.join().unwrap();
        assert!(requests[0].starts_with("PUT "));
        assert!(requests[1].starts_with("GET "));
    }
    Ok(())
}

#[test]
fn upload_snapshot_isolated_from_live_edits_and_removed_after_failure() -> Result<()> {
    struct EditingRemote {
        memory: Memory,
        source: PathBuf,
    }
    impl Remote for EditingRemote {
        fn manifest(&self) -> Result<(Manifest, Option<String>)> {
            self.memory.manifest()
        }
        fn download(&self, digest: &str) -> Result<Vec<u8>> {
            self.memory.download(digest)
        }
        fn upload(&self, _: &str, _: &[u8]) -> Result<()> {
            panic!("sync must upload file snapshots")
        }
        fn upload_file(&self, digest: &str, mut file: fs::File) -> Result<()> {
            fs::write(&self.source, b"edited during transfer")?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            ensure!(hash(&bytes) == digest, "snapshot changed with live file");
            self.memory.upload(digest, &bytes)
        }
        fn publish(&self, m: &Manifest, revision: Option<&str>) -> Result<()> {
            self.memory.publish(m, revision)
        }
    }
    let f = Fixture::new();
    let source = f.b.root.join("note.md");
    fs::write(&source, b"snapshot content")?;
    let r = EditingRemote {
        memory: Memory::default(),
        source: source.clone(),
    };
    assert!(synchronize(&f.b, &r, "x").is_err());
    assert_eq!(
        r.memory.objects.lock().unwrap()[&hash(b"snapshot content")],
        b"snapshot content"
    );
    assert!(r.memory.manifest.lock().unwrap().files.is_empty());
    assert_eq!(fs::read(&source)?, b"edited during transfer");
    assert!(!baseline_path(&f.b, "x").exists());
    let cache = baseline_path(&f.b, "x").with_extension("downloads");
    assert_eq!(fs::read_dir(&cache)?.count(), 0);
    r.memory.fail_upload.store(true, Ordering::Relaxed);
    assert!(synchronize(&f.b, &r, "x").is_err());
    assert_eq!(fs::read_dir(&cache)?.count(), 0);
    r.memory.fail_upload.store(false, Ordering::Relaxed);
    synchronize(&f.b, &r, "x")?;
    assert_eq!(
        r.memory.manifest.lock().unwrap().files["note.md"],
        hash(b"edited during transfer")
    );
    assert_eq!(fs::read_dir(cache)?.count(), 0);
    Ok(())
}

#[test]
fn upload_snapshot_rejects_stale_content_before_transport() -> Result<()> {
    let f = Fixture::new();
    let state = baseline_path(&f.b, "x");
    fs::create_dir_all(state.parent().unwrap())?;
    let cache = downloads::Cache::new(&state)?;
    let path = f.b.root.join("note.md");
    fs::write(&path, "new content")?;
    let r = Memory::default();
    assert!(
        cache
            .upload(&path, &hash(b"old content"), &r, &Cancellation::default())
            .is_err()
    );
    assert_eq!(r.uploads.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read_dir(state.with_extension("downloads"))?.count(), 0);
    Ok(())
}

#[test]
#[ignore = "writes and uploads 576 MiB; run explicitly for large-library acceptance"]
fn streamed_upload_uses_file_snapshots_for_large_library() -> Result<()> {
    struct StreamingRemote {
        uploaded: Mutex<BTreeSet<String>>,
        published: Mutex<Option<Manifest>>,
    }
    impl Remote for StreamingRemote {
        fn manifest(&self) -> Result<(Manifest, Option<String>)> {
            Ok((Manifest::default(), None))
        }
        fn download(&self, _: &str) -> Result<Vec<u8>> {
            unreachable!()
        }
        fn upload(&self, _: &str, _: &[u8]) -> Result<()> {
            panic!("must use file snapshot")
        }
        fn upload_file(&self, digest: &str, mut file: fs::File) -> Result<()> {
            let mut buffer = [0u8; 64 * 1024];
            let mut hash = Sha256::new();
            let mut bytes = 0;
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                bytes += count;
                hash.update(&buffer[..count]);
            }
            assert_eq!(bytes, 64 * 1024 * 1024);
            assert_eq!(format!("{:x}", hash.finalize()), digest);
            self.uploaded.lock().unwrap().insert(digest.into());
            Ok(())
        }
        fn publish(&self, manifest: &Manifest, revision: Option<&str>) -> Result<()> {
            assert!(revision.is_none());
            assert_eq!(manifest.files.len(), 9);
            assert_eq!(
                manifest.files.values().cloned().collect::<BTreeSet<_>>(),
                *self.uploaded.lock().unwrap()
            );
            *self.published.lock().unwrap() = Some(manifest.clone());
            Ok(())
        }
    }
    let f = Fixture::new();
    for value in 0..9u8 {
        let mut file = fs::File::create(f.b.root.join(format!("{value}.bin")))?;
        for _ in 0..1024 {
            file.write_all(&[value; 64 * 1024])?;
        }
    }
    let remote = StreamingRemote {
        uploaded: Mutex::new(BTreeSet::new()),
        published: Mutex::new(None),
    };
    assert_eq!(synchronize(&f.b, &remote, "large-upload")?.uploaded, 9);
    assert_eq!(
        snapshot(&f.b)?,
        remote.published.lock().unwrap().as_ref().unwrap().files
    );
    assert_eq!(
        fs::read_dir(baseline_path(&f.b, "large-upload").with_extension("downloads"))?.count(),
        0
    );
    Ok(())
}

#[test]
fn concurrent_scan_matches_content_and_reports_on_calling_thread() -> Result<()> {
    let f = Fixture::new();
    let mut expected = Files::new();
    let mut expected_bytes = 0;
    for index in 0..73 {
        let name = format!("文件-{index}.md");
        let body = "中文🙂".repeat(1 + index * 103);
        fs::write(f.a.root.join(&name), &body)?;
        expected.insert(name, hash(body.as_bytes()));
        expected_bytes += body.len() as u64;
    }
    // RefCell deliberately makes this callback non-Sync. No UI-facing callback
    // may migrate to a hashing worker, and progress remains monotonic.
    let events = std::cell::RefCell::new(Vec::new());
    let caller = std::thread::current().id();
    let actual = snapshot_with_progress(&f.a, Phase::Scanning, &|event| {
        assert_eq!(std::thread::current().id(), caller);
        events.borrow_mut().push(event);
    })?;
    assert_eq!(actual, expected);
    let events = events.into_inner();
    assert_eq!(events.last().unwrap().bytes, expected_bytes);
    assert_eq!(events.last().unwrap().completed, 73);
    for pair in events[1..].windows(2) {
        assert_eq!(pair[1].completed, pair[0].completed + 1);
        assert!(pair[1].bytes > pair[0].bytes);
    }
    Ok(())
}

#[test]
fn concurrent_scan_cancellation_and_removed_files_never_return_partial_manifest() -> Result<()> {
    let f = Fixture::new();
    for index in 0..40 {
        fs::write(f.a.root.join(format!("{index}.md")), "content")?;
    }
    let token = Cancellation::default();
    let result = snapshot_cancellable(
        &f.a,
        Phase::Scanning,
        &|event| {
            if event.completed == 1 {
                token.request();
            }
        },
        &token,
    );
    assert!(is_cancelled(&result.unwrap_err()));
    assert_eq!(snapshot(&f.a)?.len(), 40);
    let result = snapshot_with_progress(&f.a, Phase::Scanning, &|event| {
        if event.total == 40 && event.completed == 0 {
            for index in 0..40 {
                fs::remove_file(f.a.root.join(format!("{index}.md"))).unwrap();
            }
        }
    });
    assert!(result.is_err());
    assert!(snapshot(&f.a)?.is_empty());
    Ok(())
}

#[test]
fn operations_coordinate_across_recovery_directories_and_release_failed_attempts() -> Result<()> {
    let f = Fixture::new();
    let other = Vault::open(&f.a.root, f.root.join("other-recovery"))?;
    let remote = Memory::default();
    fs::write(f.a.root.join("note.md"), b"old")?;
    apply_bytes(&f.a, "note.md", Some(&hash(b"old")), Some(b"new"))?;
    let entry = recovery::inventory(&f.a)?.entries.remove(0);
    let owner = lock_operation(&f.a)?;
    assert!(synchronize(&other, &remote, "other-profile").is_err());
    assert!(recovery::restore_copy(&other, &entry, &[]).is_err());
    assert_eq!(remote.uploads.load(Ordering::Relaxed), 0);
    assert_eq!(remote.revision.load(Ordering::Relaxed), 0);
    assert!(!f.a.root.join("note 同步恢复.md").exists());
    // A separate vault remains independent, even in the same recovery directory.
    drop(lock_operation(&f.b)?);
    drop(owner);
    // A failed second lock acquisition must release its already-acquired device lock.
    let restored = recovery::restore_copy(&other, &entry, &[])?;
    assert_eq!(fs::read(f.a.root.join(restored.relative))?, b"old");
    synchronize(&other, &remote, "other-profile")?;
    assert_eq!(fs::read(f.a.root.join("note.md"))?, b"new");
    assert_eq!(fs::read(f.a.root.join(entry.backup))?, b"old");
    drop(lock_operation(&f.a)?);
    Ok(())
}

#[cfg(unix)]
#[test]
fn canonical_vault_alias_uses_the_same_operation_lock() -> Result<()> {
    let f = Fixture::new();
    let alias = f.root.join("alias");
    std::os::unix::fs::symlink(&f.a.root, &alias)?;
    let other = Vault::open(alias, f.root.join("alias-recovery"))?;
    let owner = lock_operation(&f.a)?;
    assert!(lock_operation(&other).is_err());
    drop(owner);
    drop(lock_operation(&other)?);
    Ok(())
}
