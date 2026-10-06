use super::*;
use core::prelude::v1::test;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;

type Reply = (&'static str, String, u16, String, String);
fn row(method: &'static str, path: &str, code: u16, body: String, headers: &str) -> Reply {
    (method, path.into(), code, body, headers.into())
}
fn lock() -> String {
    "<d:prop xmlns:d=\"DAV:\"><d:lockdiscovery><d:activelock><d:lockscope><d:exclusive/></d:lockscope><d:locktype><d:write/></d:locktype><d:depth>infinity</d:depth><d:locktoken><d:href>urn:uuid:fixture</d:href></d:locktoken><d:lockroot><d:href>$ROOTinkstone/</d:href></d:lockroot><d:timeout>Second-30</d:timeout></d:activelock></d:lockdiscovery></d:prop>".into()
}
fn refresh() -> Reply {
    row("LOCK", "/inkstone/", 200, lock(), "")
}
fn manifest(version: u32, digest: &str) -> String {
    let mut value = serde_json::json!({"version": version, "files": {"remote.md": digest}});
    if version == 2 {
        value["generation"] = serde_json::json!(1);
    }
    value.to_string()
}
fn xml(objects: &std::collections::BTreeMap<String, &'static str>) -> String {
    let rows: String = objects.iter().map(|(digest, bytes)| format!("<d:response><d:href>{digest}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{}</d:getcontentlength></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",bytes.len())).collect();
    format!("<d:multistatus xmlns:d=\"DAV:\">{rows}</d:multistatus>")
}
fn reads(manifest: &str, tag: &str) -> Vec<Reply> {
    vec![
        row("GET", "/inkstone-v1/manifest.json", 404, String::new(), ""),
        row(
            "GET",
            "/inkstone/manifest.json",
            200,
            manifest.into(),
            &format!("ETag: \"{tag}\"\r\n"),
        ),
    ]
}
fn snapshot(
    manifest: &str,
    objects: &std::collections::BTreeMap<String, &'static str>,
    tag: &str,
) -> Vec<Reply> {
    let mut rows = reads(manifest, tag);
    rows.push(row("PROPFIND", "/inkstone/objects/", 207, xml(objects), ""));
    rows.extend(reads(manifest, tag));
    rows
}
fn server(execute: bool) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let retained = format!("{:x}", Sha256::digest(b"current"));
    let candidate = format!("{:x}", Sha256::digest(b"old"));
    let mut objects = std::collections::BTreeMap::from([
        (retained.clone(), "current"),
        (candidate.clone(), "old"),
    ]);
    let before = manifest(1, &retained);
    let after = manifest(2, &retained);
    let mut replies = snapshot(&before, &objects, "before");
    if execute {
        replies.push(row(
            "LOCK",
            "/inkstone/",
            200,
            lock(),
            "Lock-Token: <urn:uuid:fixture>\r\n",
        ));
        replies.extend(snapshot(&before, &objects, "before"));
        for (digest, bytes) in &objects {
            replies.push(refresh());
            replies.push(row(
                "GET",
                &format!("/inkstone/objects/{digest}"),
                200,
                (*bytes).into(),
                "",
            ));
        }
        replies.push(refresh());
        replies.extend(reads(&before, "before"));
        replies.push(row(
            "PUT",
            "/inkstone/manifest.json",
            204,
            String::new(),
            "",
        ));
        replies.extend(reads(&after, "after"));
        replies.extend(snapshot(&after, &objects, "after"));
        replies.extend([refresh(), refresh()]);
        replies.extend(snapshot(&after, &objects, "after"));
        replies.push(refresh());
        replies.push(row(
            "DELETE",
            &format!("/inkstone/objects/{candidate}"),
            204,
            String::new(),
            "",
        ));
        replies.push(row(
            "HEAD",
            &format!("/inkstone/objects/{candidate}"),
            404,
            String::new(),
            "",
        ));
        objects.remove(&candidate);
        replies.push(refresh());
        replies.extend(snapshot(&after, &objects, "after"));
        replies.push(row("UNLOCK", "/inkstone/", 204, String::new(), ""));
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let base = url.clone();
    let task = std::thread::spawn(move || {
        let mut methods = vec![];
        for (method, path, code, body, headers) in replies {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("missing {method} {path}: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = vec![];
            while !bytes.ends_with(b"\r\n\r\n") {
                assert!(bytes.len() < 64 * 1024);
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(
                request.starts_with(&format!("{method} {path} HTTP/1.1\r\n")),
                "unexpected request route"
            );
            let length = request
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(length < 64 * 1024);
            stream.read_exact(&mut vec![0; length]).unwrap();
            methods.push(method.into());
            let body = body.replace("$ROOT", &base);
            write!(stream, "HTTP/1.1 {code} Test\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        methods
    });
    (url, task)
}

#[test]
fn invalidation_cancels_running_work_without_accepting_stale_results() {
    let key = (1, [1; 32]);
    let new = (2, [2; 32]);
    let cancellation = Arc::new(sync::Cancellation::default());
    let mut state = State::default();
    state.request = 7;
    state.key = Some(key);
    state.run = Some(CleanupRun {
        request: 7,
        key,
        cancellation: cancellation.clone(),
    });
    state.invalidate();
    assert!(cancellation.is_requested());
    assert!(state.is_running());
    state.message = "new connection".into();
    assert!(!state.finish_run(6, key, new, "foreign".into()));
    assert!(state.is_running());
    assert!(!state.finish_run(7, key, new, "old result".into()));
    assert!(!state.is_running());
    assert_eq!(state.message, "new connection");
    state.finish_preview(7, key, new, Err("old preview".into()));
    assert_eq!(state.message, "new connection");
}

fn exercise_ui(cx: &mut TestAppContext, execute: bool, stale: bool, manual_save: bool) {
    cx.update(gpui_kit::init);
    let (url, server) = server(execute);
    let root = std::env::temp_dir().join(format!(
        "inkstone-cloud-cleanup-ui-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("vault")).unwrap();
    std::fs::write(root.join("vault/note.md"), "original").unwrap();
    let handle = cx.add_window(Workspace::new);
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            w.vault = Some(Vault::open(root.join("vault"), root.join("recovery")).unwrap());
            w.add_tab("note.md".into(), Some("original".into()), false, window, cx);
            w.tabs[0].save.editor.update(cx, |editor, cx| {
                editor.set_value("unsaved 中文 👩‍💻", window, cx)
            });
            w.flush_document_views(window, cx);
            w.ui.cloud_sync
                .url
                .update(cx, |input, cx| input.set_value(url, window, cx));
            w.ui.cloud_sync
                .username
                .update(cx, |input, cx| input.set_value("test user", window, cx));
            w.ui.cloud_sync
                .password
                .update(cx, |input, cx| input.set_value("test secret", window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    let prefs = std::fs::read(root.join("vault/.inkstone-workspace.json")).ok();
    handle
        .update(cx, |w, _, cx| {
            w.refresh_cloud_cleanup(cx);
            assert!(w.ui.cloud_sync.cleanup.loading);
            assert_eq!(w.file_writes.pending(), 0);
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(
                w.ui.cloud_sync.cleanup.preview.as_ref().unwrap().candidates,
                1
            );
            w.execute_cloud_cleanup(cx); // Confirmation is required.
            assert!(w.ui.cloud_sync.cleanup.run.is_none());
            let busy = w.file_writes.begin();
            w.request_cloud_cleanup(cx);
            assert!(!w.ui.cloud_sync.cleanup.confirmation);
            w.file_writes.finish(busy);
            w.request_cloud_cleanup(cx);
            assert!(w.ui.cloud_sync.cleanup.confirmation);
            w.execute_cloud_cleanup(cx);
            w.execute_cloud_cleanup(cx); // Repeated activation cannot start another job.
            assert!(w.ui.cloud_sync.cleanup.is_running());
            assert_eq!(w.file_writes.pending(), 1);
            assert!(w.cloud_settings_disabled());
            assert!(
                !w.file_writes.operation_active(),
                "cloud cleanup must not block explicit local saves"
            );
            assert!(!w.save_cloud_settings(cx));
            w.request_cloud_sync(window, cx);
            assert!(!w.ui.cloud_sync.is_pending());
            if manual_save {
                w.save_all(window, cx);
                assert!(w.tabs[0].save.persistence.is_saving());
            }
            if !execute {
                w.stop_cloud_cleanup(cx);
                w.stop_cloud_cleanup(cx);
                assert_eq!(w.file_writes.pending(), 1);
                if stale {
                    w.generation += 1;
                    w.ui.cloud_sync.cleanup.message = "new vault".into();
                }
            }
        })
        .unwrap();
    cx.run_until_parked();
    let methods = server.join().unwrap();
    assert_eq!(
        methods
            .iter()
            .filter(|method| method.as_str() == "DELETE")
            .count(),
        usize::from(execute)
    );
    handle
        .update(cx, |w, window, cx| {
            assert_eq!(w.file_writes.pending(), 0);
            assert!(!w.ui.cloud_sync.cleanup.is_running());
            let message = &w.ui.cloud_sync.cleanup.message;
            if stale {
                assert_eq!(message, "new vault");
            } else if execute {
                assert!(
                    message.contains("已确认清理 1 个") && message.contains("3 字节"),
                    "{message}"
                );
            } else {
                assert!(message.contains("已停止云端清理"), "{message}");
            }
            assert!(w.ui.cloud_sync.cleanup.preview.is_none());
            w.flush_document_views(window, cx);
            assert_eq!(w.tabs[0].save.persistence.is_dirty(), !manual_save);
            assert_eq!(
                w.tabs[0].save.editor.read(cx).value().as_ref(),
                "unsaved 中文 👩‍💻"
            );
            assert!(w.ui.prefs.webdav.url.is_empty());
            assert!(w.ui.prefs.webdav.username.is_empty());
        })
        .unwrap();
    assert_eq!(
        std::fs::read(root.join("vault/note.md")).unwrap(),
        if manual_save {
            "unsaved 中文 👩‍💻".as_bytes()
        } else {
            b"original"
        }
    );
    assert_eq!(
        std::fs::read(root.join("vault/.inkstone-workspace.json")).ok(),
        prefs
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn cleanup_http_success_preserves_unsaved_editor_and_unpersisted_settings(cx: &mut TestAppContext) {
    exercise_ui(cx, true, false, false);
}
#[gpui::test]
fn cancelling_cleanup_retains_ticket_until_worker_completion(cx: &mut TestAppContext) {
    exercise_ui(cx, false, false, false);
}
#[gpui::test]
fn old_cleanup_completion_only_releases_its_task_ticket(cx: &mut TestAppContext) {
    exercise_ui(cx, false, true, false);
}

#[gpui::test]
fn explicit_note_save_remains_available_during_cloud_cleanup(cx: &mut TestAppContext) {
    exercise_ui(cx, false, false, true);
}
