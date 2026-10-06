//! Stateful HTTP model: conditional mutation and lock evaluation are serialized
//! under one server mutex. Tests exercise real clients, not scripted responses.
//! This models a conforming server; it does not certify a real WebDAV provider.
use super::*;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, mpsc};

#[derive(Default)]
struct State {
    manifest: Option<Vec<u8>>,
    objects: BTreeMap<String, Vec<u8>>,
    lock: Option<String>,
    next_lock: usize,
    expire_before_delete: bool,
    lose_delete_response: bool,
    events: Vec<(String, String, u16)>,
    pause_after: Option<(String, mpsc::Sender<()>, mpsc::Receiver<()>)>,
}
impl State {
    fn etag(&self) -> Option<String> {
        self.manifest
            .as_ref()
            .map(|bytes| format!("\"{}\"", hash(bytes)))
    }
}
struct Server {
    url: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    task: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}/dav/", listener.local_addr()?);
        let manifest = Manifest {
            files: BTreeMap::from([("note.md".into(), hash(b"base"))]),
            ..Manifest::default()
        };
        let state = Arc::new(Mutex::new(State {
            manifest: Some(serde_json::to_vec(&manifest)?),
            objects: BTreeMap::from([
                (hash(b"base"), b"base".to_vec()),
                (hash(b"orphan"), b"orphan".to_vec()),
            ]),
            ..State::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = state.clone();
        let stopping = stop.clone();
        let root = url.clone();
        let task = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(error) => panic!("stateful test accept: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let request = Request::read(&mut stream).unwrap();
                let mut state = shared.lock().unwrap();
                let response = respond(&root, &mut state, &request);
                let pause = if response.code < 300
                    && state
                        .pause_after
                        .as_ref()
                        .is_some_and(|(method, _, _)| *method == request.method)
                {
                    state.pause_after.take()
                } else {
                    None
                };
                state
                    .events
                    .push((request.method, request.path, response.code));
                drop(state);
                if let Some((_, reached, resume)) = pause {
                    reached.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(15)).unwrap();
                    // The parent killed the client after the mutation but before
                    // its response. Do not write to the deliberately dead socket.
                    continue;
                }
                if response.code == 0 {
                    continue;
                }
                write!(
                    stream,
                    "HTTP/1.1 {} Test\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    response.code,
                    response.headers,
                    response.body.len()
                )
                .unwrap();
                stream.write_all(&response.body).unwrap();
            }
        });
        Ok(Self {
            url,
            state,
            stop,
            task: Some(task),
        })
    }
    fn client(&self) -> Result<WebDav> {
        WebDav::new(
            &Settings {
                url: self.url.clone(),
                ..Settings::default()
            },
            "",
        )
    }
    fn assert_references_exist(&self) {
        let state = self.state.lock().unwrap();
        let manifest: Manifest = serde_json::from_slice(state.manifest.as_ref().unwrap()).unwrap();
        for digest in manifest.files.values() {
            let bytes = &state.objects[digest];
            assert_eq!(hash(bytes), *digest);
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.take() {
            let result = task.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}
struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
impl Request {
    fn read(stream: &mut TcpStream) -> Result<Self> {
        let mut raw = vec![];
        while !raw.ends_with(b"\r\n\r\n") {
            ensure!(raw.len() < 64 * 1024, "oversized test header");
            let mut byte = [0];
            stream.read_exact(&mut byte)?;
            raw.push(byte[0]);
        }
        let text = String::from_utf8(raw)?;
        let mut lines = text.lines();
        let mut first = lines.next().unwrap().split_whitespace();
        let method = first.next().unwrap().into();
        let path = first.next().unwrap().into();
        let headers: BTreeMap<String, String> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_lowercase(), value.trim().into()))
            .collect();
        let length = headers
            .get("content-length")
            .map(|length| length.parse::<usize>())
            .transpose()?
            .unwrap_or(0);
        ensure!(length <= 1024 * 1024, "oversized fixture");
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        Ok(Self {
            method,
            path,
            headers,
            body,
        })
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}
struct Response {
    code: u16,
    headers: String,
    body: Vec<u8>,
}
impl Response {
    fn code(code: u16) -> Self {
        Self {
            code,
            headers: String::new(),
            body: vec![],
        }
    }
    fn body(code: u16, body: Vec<u8>) -> Self {
        Self {
            code,
            headers: String::new(),
            body,
        }
    }
}
fn lock_reply(root: &str, token: &str, created: bool) -> Response {
    use super::super::test_support;
    Response {
        code: 200,
        headers: if created {
            format!("Lock-Token: {token}\r\n")
        } else {
            String::new()
        },
        body: test_support::body()
            .replace(
                &test_support::TOKEN[1..test_support::TOKEN.len() - 1],
                &token[1..token.len() - 1],
            )
            .replace("$ROOT", &format!("{root}inkstone/"))
            .into_bytes(),
    }
}
fn respond(root: &str, state: &mut State, request: &Request) -> Response {
    let mut path = request.path.strip_prefix("/dav/").unwrap();
    if path == "inkstone-v1/manifest.json" {
        return Response::code(404);
    }
    if request.method == "DELETE" && state.expire_before_delete {
        state.expire_before_delete = false;
        state.lock = None;
    }
    let condition = state
        .lock
        .as_ref()
        .map(|token| format!("<{root}inkstone/> ({token})"));
    if request.header("if").is_some() && request.header("if") != condition.as_deref() {
        return Response::code(412);
    }
    if path == "inkstone/" && request.method == "LOCK" {
        if request.body.is_empty() {
            return match state
                .lock
                .as_ref()
                .filter(|_| request.header("if").is_some())
            {
                Some(token) => lock_reply(root, token, false),
                None => Response::code(412),
            };
        }
        if state.lock.is_some() {
            return Response::code(423);
        }
        assert_eq!(request.header("depth"), Some("infinity"));
        assert_eq!(request.header("if-match"), Some("*"));
        state.next_lock += 1;
        let token = format!("<urn:uuid:test-lock-{}>", state.next_lock);
        let reply = lock_reply(root, &token, true);
        state.lock = Some(token);
        return reply;
    }
    if path == "inkstone/" && request.method == "UNLOCK" {
        if state.lock.is_some() && request.header("lock-token") == state.lock.as_deref() {
            state.lock = None;
            return Response::code(204);
        }
        return Response::code(409);
    }
    if matches!(request.method.as_str(), "PUT" | "DELETE")
        && state.lock.is_some()
        && request.header("if") != condition.as_deref()
    {
        return Response::code(423);
    }
    if path == "inkstone/manifest.json" {
        if request.method == "GET" {
            return match &state.manifest {
                Some(bytes) => Response {
                    code: 200,
                    headers: format!("ETag: {}\r\n", state.etag().unwrap()),
                    body: bytes.clone(),
                },
                None => Response::code(404),
            };
        }
        if request.method == "PUT" {
            let permitted = match state.etag() {
                Some(etag) => request.header("if-match") == Some(etag.as_str()),
                None => request.header("if-none-match") == Some("*"),
            };
            if !permitted {
                return Response::code(412);
            }
            state.manifest = Some(request.body.clone());
            return Response::code(204);
        }
    }
    if path == "inkstone/objects/" && request.method == "PROPFIND" {
        assert_eq!(request.header("depth"), Some("1"));
        let rows: String = state.objects.iter().map(|(digest, bytes)| format!("<d:response><d:href>{digest}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{}</d:getcontentlength></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>", bytes.len())).collect();
        return Response::body(
            207,
            format!("<d:multistatus xmlns:d=\"DAV:\">{rows}</d:multistatus>").into_bytes(),
        );
    }
    if let Some(digest) = path.strip_prefix("inkstone/objects/") {
        path = digest;
        assert!(valid_hash(path));
        return match request.method.as_str() {
            "GET" => state
                .objects
                .get(path)
                .map(|bytes| Response::body(200, bytes.clone()))
                .unwrap_or_else(|| Response::code(404)),
            "HEAD" => Response::code(if state.objects.contains_key(path) {
                200
            } else {
                404
            }),
            "PUT" => {
                assert_eq!(request.header("if-none-match"), Some("*"));
                if state.objects.contains_key(path) {
                    Response::code(412)
                } else {
                    state.objects.insert(path.into(), request.body.clone());
                    Response::code(201)
                }
            }
            "DELETE" => {
                assert!(request.header("if").is_some());
                if state.objects.remove(path).is_none() {
                    return Response::code(404);
                }
                if state.lose_delete_response {
                    state.lose_delete_response = false;
                    Response::code(0)
                } else {
                    Response::code(204)
                }
            }
            _ => panic!("unexpected object method"),
        };
    }
    panic!("unexpected test route: {} {}", request.method, request.path)
}

struct Vaults {
    root: PathBuf,
    a: Vault,
    b: Vault,
}
impl Vaults {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!("inkstone-cloud-race-{}", unique_id()));
        fs::create_dir_all(root.join("a"))?;
        fs::create_dir_all(root.join("b"))?;
        Ok(Self {
            a: Vault::open(root.join("a"), root.join("recovery"))?,
            b: Vault::open(root.join("b"), root.join("recovery"))?,
            root,
        })
    }
}
impl Drop for Vaults {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct PausedWriter {
    remote: WebDav,
    uploaded: mpsc::Sender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
}
impl Remote for PausedWriter {
    fn manifest(&self) -> Result<(Manifest, Option<String>)> {
        self.remote.manifest()
    }
    fn download(&self, digest: &str) -> Result<Vec<u8>> {
        self.remote.download(digest)
    }
    fn upload(&self, digest: &str, bytes: &[u8]) -> Result<()> {
        self.remote.upload(digest, bytes)
    }
    fn upload_file(&self, digest: &str, file: fs::File) -> Result<()> {
        self.remote.upload_file(digest, file)?;
        self.uploaded.send(())?;
        self.resume
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))?;
        Ok(())
    }
    fn publish(&self, manifest: &Manifest, revision: Option<&str>) -> Result<()> {
        self.remote.publish(manifest, revision)
    }
}

#[test]
fn uploaded_but_unpublished_content_is_retried_after_fenced_cleanup() -> Result<()> {
    let server = Server::new()?;
    let vaults = Vaults::new()?;
    let client = server.client()?;
    synchronize(&vaults.a, &client, "race")?;
    let baseline = baseline_path(&vaults.a, "race");
    let saved = fs::read(&baseline)?;
    fs::write(vaults.a.root.join("note.md"), "pending 中文 edit")?;
    let (uploaded, received) = mpsc::channel();
    let (resume, gate) = mpsc::channel();
    let writer = PausedWriter {
        remote: server.client()?,
        uploaded,
        resume: Mutex::new(gate),
    };
    std::thread::scope(|scope| -> Result<()> {
        let task = scope.spawn(|| synchronize(&vaults.a, &writer, "race"));
        received.recv_timeout(Duration::from_secs(5))?;
        let (before, revision) = client.manifest()?;
        let preview = client.preview_cloud_cleanup()?;
        assert_eq!(preview.candidates, 2);
        let checked = client.prepare_cloud_cleanup(&preview, &Cancellation::default())?;
        let competing = server.client()?;
        assert!(competing.lock_maintenance().is_err());
        assert!(competing.upload(&hash(b"blocked"), b"blocked").is_err());
        assert!(competing.publish(&before, revision.as_deref()).is_err());
        assert_eq!(competing.download(&hash(b"base"))?, b"base");
        assert_eq!(checked.execute(&Cancellation::default())?.removed, 2);
        let (after, new_revision) = client.manifest()?;
        assert_eq!(before.files, after.files);
        assert_ne!(revision, new_revision); // Content-derived ETags, not a revision counter.
        resume.send(())?;
        assert!(task.join().unwrap().is_err());
        Ok(())
    })?;
    assert_eq!(fs::read(&baseline)?, saved);
    assert_eq!(
        fs::read_to_string(vaults.a.root.join("note.md"))?,
        "pending 中文 edit"
    );
    assert!(
        !server
            .state
            .lock()
            .unwrap()
            .objects
            .contains_key(&hash("pending 中文 edit".as_bytes()))
    );
    synchronize(&vaults.a, &client, "race")?;
    synchronize(&vaults.b, &server.client()?, "race")?;
    assert_eq!(
        fs::read(vaults.a.root.join("note.md"))?,
        fs::read(vaults.b.root.join("note.md"))?
    );
    assert_eq!(client.manifest()?.0.generation, Some(1));
    server.assert_references_exist();
    let state = server.state.lock().unwrap();
    assert!(
        state
            .events
            .iter()
            .any(|(method, path, code)| method == "PUT"
                && path.ends_with("manifest.json")
                && *code == 412)
    );
    Ok(())
}

#[test]
fn expired_cleanup_cannot_delete_or_unlock_a_new_maintenance_session() -> Result<()> {
    let server = Server::new()?;
    let first = server.client()?;
    let second = server.client()?;
    let checked =
        first.prepare_cloud_cleanup(&first.preview_cloud_cleanup()?, &Cancellation::default())?;
    server.state.lock().unwrap().lock = None;
    let replacement =
        second.prepare_cloud_cleanup(&second.preview_cloud_cleanup()?, &Cancellation::default())?;
    let token = server.state.lock().unwrap().lock.clone();
    let error = checked.execute(&Cancellation::default()).unwrap_err();
    let failure = error.downcast_ref::<CloudCleanupFailure>().unwrap();
    assert_eq!(failure.completed.removed, 0);
    assert!(failure.lock_release_failed);
    assert_eq!(server.state.lock().unwrap().lock, token);
    assert_eq!(replacement.execute(&Cancellation::default())?.removed, 1);
    assert_eq!(second.manifest()?.0.generation, Some(2));
    server.assert_references_exist();
    Ok(())
}

#[test]
fn expiry_between_refresh_and_delete_is_enforced_by_the_server() -> Result<()> {
    let server = Server::new()?;
    let client = server.client()?;
    let checked =
        client.prepare_cloud_cleanup(&client.preview_cloud_cleanup()?, &Cancellation::default())?;
    server.state.lock().unwrap().expire_before_delete = true;
    let error = checked.execute(&Cancellation::default()).unwrap_err();
    let failure = error.downcast_ref::<CloudCleanupFailure>().unwrap();
    assert_eq!(failure.completed.removed, 0);
    assert!(!failure.uncertain);
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .objects
            .contains_key(&hash(b"orphan"))
    );
    server.assert_references_exist();
    let checked =
        client.prepare_cloud_cleanup(&client.preview_cloud_cleanup()?, &Cancellation::default())?;
    assert_eq!(checked.execute(&Cancellation::default())?.removed, 1);
    server.assert_references_exist();
    Ok(())
}

#[test]
fn lost_response_after_real_deletion_converges_from_a_fresh_preview() -> Result<()> {
    let server = Server::new()?;
    server
        .state
        .lock()
        .unwrap()
        .objects
        .insert(hash(b"second orphan"), b"second orphan".to_vec());
    let client = server.client()?;
    let checked =
        client.prepare_cloud_cleanup(&client.preview_cloud_cleanup()?, &Cancellation::default())?;
    server.state.lock().unwrap().lose_delete_response = true;
    let error = checked.execute(&Cancellation::default()).unwrap_err();
    let failure = error.downcast_ref::<CloudCleanupFailure>().unwrap();
    assert!(failure.uncertain);
    assert_eq!(failure.completed.removed, 0);
    assert!(server.state.lock().unwrap().lock.is_none());
    server.assert_references_exist();
    let preview = client.preview_cloud_cleanup()?;
    assert_eq!(preview.candidates, 1);
    let checked = client.prepare_cloud_cleanup(&preview, &Cancellation::default())?;
    assert_eq!(checked.execute(&Cancellation::default())?.removed, 1);
    assert_eq!(client.preview_cloud_cleanup()?.candidates, 0);
    server.assert_references_exist();
    Ok(())
}

struct PausedReader {
    remote: WebDav,
    read: mpsc::Sender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
}
impl Remote for PausedReader {
    fn manifest(&self) -> Result<(Manifest, Option<String>)> {
        let snapshot = self.remote.manifest()?;
        self.read.send(())?;
        self.resume
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))?;
        Ok(snapshot)
    }
    fn download(&self, digest: &str) -> Result<Vec<u8>> {
        self.remote.download(digest)
    }
    fn upload(&self, digest: &str, bytes: &[u8]) -> Result<()> {
        self.remote.upload(digest, bytes)
    }
    fn publish(&self, manifest: &Manifest, revision: Option<&str>) -> Result<()> {
        self.remote.publish(manifest, revision)
    }
}

#[test]
fn reader_of_retired_snapshot_keeps_local_state_then_retries_current_manifest() -> Result<()> {
    let server = Server::new()?;
    let vaults = Vaults::new()?;
    let client = server.client()?;
    synchronize(&vaults.b, &client, "reader-race")?;
    let baseline = baseline_path(&vaults.b, "reader-race");
    let saved = fs::read(&baseline)?;
    let publish = |text: &[u8]| -> Result<()> {
        let (mut manifest, revision) = client.manifest()?;
        let digest = hash(text);
        client.upload(&digest, text)?;
        manifest.files.insert("note.md".into(), digest);
        client.publish(&manifest, revision.as_deref())
    };
    publish(b"intermediate")?;
    let (read, received) = mpsc::channel();
    let (resume, gate) = mpsc::channel();
    let reader = PausedReader {
        remote: server.client()?,
        read,
        resume: Mutex::new(gate),
    };
    std::thread::scope(|scope| -> Result<()> {
        let task = scope.spawn(|| synchronize(&vaults.b, &reader, "reader-race"));
        received.recv_timeout(Duration::from_secs(5))?;
        publish(b"latest")?;
        let checked = client
            .prepare_cloud_cleanup(&client.preview_cloud_cleanup()?, &Cancellation::default())?;
        assert_eq!(checked.execute(&Cancellation::default())?.removed, 3);
        resume.send(())?;
        assert!(task.join().unwrap().is_err());
        Ok(())
    })?;
    assert_eq!(fs::read(vaults.b.root.join("note.md"))?, b"base");
    assert_eq!(fs::read(&baseline)?, saved);
    server.assert_references_exist();
    synchronize(&vaults.b, &client, "reader-race")?;
    assert_eq!(fs::read(vaults.b.root.join("note.md"))?, b"latest");
    assert_eq!(read_baseline(&baseline)?.generation, Some(1));
    server.assert_references_exist();
    Ok(())
}

#[test]
fn killed_cloud_cleanup_restarts_from_remote_state() -> Result<()> {
    const URL: &str = "INKSTONE_TEST_KILLED_CLOUD_CLEANUP_URL";
    const TEST: &str = "vault::sync::webdav::cleanup::stateful_tests::killed_cloud_cleanup_restarts_from_remote_state";
    if let Ok(url) = std::env::var(URL) {
        let client = WebDav::new(
            &Settings {
                url,
                ..Settings::default()
            },
            "",
        )?;
        let preview = client.preview_cloud_cleanup()?;
        client
            .prepare_cloud_cleanup(&preview, &Cancellation::default())?
            .execute(&Cancellation::default())?;
        return Ok(());
    }
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let spawn = |url: &str| -> Result<Child> {
        Ok(Child(
            std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", TEST])
                .env(URL, url)
                .stdout(std::process::Stdio::null())
                .spawn()?,
        ))
    };
    // Each checkpoint is after server mutation, before acknowledgement. No
    // destructor/UNLOCK from the killed client can repair the remote state.
    for method in ["LOCK", "PUT", "DELETE"] {
        let server = Server::new()?;
        let client = server.client()?;
        client.upload(&hash(b"another orphan"), b"another orphan")?;
        let vaults = Vaults::new()?;
        synchronize(&vaults.b, &client, "killed-cleanup")?;
        let baseline = baseline_path(&vaults.b, "killed-cleanup");
        let saved = fs::read(&baseline)?;
        fs::write(vaults.b.root.join("note.md"), "本地未同步修改 📝")?;
        let (reached, received) = mpsc::channel();
        let (resume, gate) = mpsc::channel();
        server.state.lock().unwrap().pause_after = Some((method.into(), reached, gate));
        let mut child = spawn(&server.url)?;
        received.recv_timeout(Duration::from_secs(10))?;
        child.0.kill()?;
        assert!(!child.0.wait()?.success());
        resume.send(())?;
        server.assert_references_exist();
        {
            let state = server.state.lock().unwrap();
            assert!(
                state.lock.is_some(),
                "killed client must leave its lease behind"
            );
            assert!(!state.events.iter().any(|(m, _, _)| m == "UNLOCK"));
            let manifest: Manifest = serde_json::from_slice(state.manifest.as_ref().unwrap())?;
            assert_eq!(
                manifest.generation,
                if method == "LOCK" { None } else { Some(1) }
            );
            assert_eq!(state.objects.len(), if method == "DELETE" { 2 } else { 3 });
        }
        // A new process cannot bypass the abandoned lease before the server
        // expires it. Explicit model expiry avoids timing-dependent sleeps.
        assert!(client.lock_maintenance().is_err());
        server.state.lock().unwrap().lock = None;
        let mut restarted = spawn(&server.url)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = restarted.0.try_wait()? {
                assert!(status.success(), "restart failed after {method}");
                break;
            }
            ensure!(std::time::Instant::now() < deadline, "restart timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        server.assert_references_exist();
        assert_eq!(client.preview_cloud_cleanup()?.candidates, 0);
        assert!(server.state.lock().unwrap().lock.is_none());
        assert_eq!(
            fs::read_to_string(vaults.b.root.join("note.md"))?,
            "本地未同步修改 📝"
        );
        assert_eq!(fs::read(&baseline)?, saved);
    }
    Ok(())
}
