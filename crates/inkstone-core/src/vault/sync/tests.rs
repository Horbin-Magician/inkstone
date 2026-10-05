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
        "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "HTTP/1.1 412 Precondition Failed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    ]);
    let dav = WebDav::new(&settings, "pass")?;
    assert!(dav.manifest()?.1.is_none());
    dav.publish(&Manifest::default(), None)?;
    assert!(dav.publish(&Manifest::default(), Some("\"old\"")).is_err());
    let requests = thread.join().unwrap();
    assert!(requests[0].starts_with("GET /dav/inkstone-v1/manifest.json"));
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
        let (settings, thread) = server(vec![reply]);
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
                    username: String::new()
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
    apply(
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
    apply(&f.a, "note.md", Some(&hash(b"downloaded")), None)?;
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
