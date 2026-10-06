use super::*;
use std::net::TcpListener;

const TOKEN: &str = "<urn:uuid:19c78611-2fb5-41ab-af92-4ca641921e95>";
fn body() -> String {
    format!(
        r#"<d:prop xmlns:d="DAV:"><d:lockdiscovery><d:activelock><d:lockscope><d:exclusive/></d:lockscope><d:locktype><d:write/></d:locktype><d:depth>infinity</d:depth><d:locktoken><d:href>{}</d:href></d:locktoken><d:lockroot><d:href>$ROOT</d:href></d:lockroot><d:timeout>Second-30</d:timeout></d:activelock></d:lockdiscovery></d:prop>"#,
        &TOKEN[1..TOKEN.len() - 1]
    )
}
fn grant(body: String) -> (u16, String, String) {
    (200, format!("Lock-Token: {TOKEN}\r\n"), body)
}
fn reply(code: u16) -> (u16, String, String) {
    (code, String::new(), String::new())
}
fn server(replies: Vec<(u16, String, String)>) -> (WebDav, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/dav/", listener.local_addr().unwrap());
    let remote = WebDav::new(
        &Settings {
            url: url.clone(),
            ..Default::default()
        },
        "",
    )
    .unwrap();
    let task = std::thread::spawn(move || {
        let mut requests = vec![];
        for (code, headers, body) in replies {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => panic!("missing maintenance request: {e}"),
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
            let mut request = String::from_utf8(bytes).unwrap();
            let length = request
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(length < 64 * 1024);
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).unwrap();
            request.push_str(std::str::from_utf8(&bytes).unwrap());
            requests.push(request);
            let body = body.replace("$ROOT", &format!("{url}inkstone/"));
            write!(stream, "HTTP/1.1 {code} Test\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    (remote, task)
}

#[test]
fn acquire_refresh_and_release_use_server_lock_conditions() -> Result<()> {
    let (remote, task) = server(vec![
        grant(body()),
        (200, String::new(), body().replace("Second-30", "Second-20")),
        reply(204),
    ]);
    let mut lock = remote.lock_maintenance()?;
    assert_eq!(lock.seconds(), 30);
    lock.refresh()?;
    assert_eq!(lock.seconds(), 20);
    lock.release()?;
    let requests = task.join().unwrap();
    let acquire = requests[0].to_lowercase();
    assert!(acquire.starts_with("lock /dav/inkstone/ http/1.1\r\n"));
    assert!(acquire.contains("depth: infinity\r\n"));
    assert!(acquire.contains("if-match: *\r\n"));
    assert!(acquire.contains("timeout: second-300\r\n"));
    assert!(requests[0].ends_with(LOCK_INFO));
    let refresh = requests[1].to_lowercase();
    assert!(refresh.contains(&format!("if: <{}inkstone/> ({TOKEN})\r\n", remote.root)));
    assert!(!refresh.contains("depth:"));
    assert!(refresh.ends_with("\r\n\r\n"));
    let release = requests[2].to_lowercase();
    assert!(release.starts_with("unlock /dav/inkstone/ http/1.1\r\n"));
    assert!(release.contains(&format!("lock-token: {TOKEN}\r\n")));
    assert!(!release.contains("timeout:"));
    Ok(())
}

#[test]
fn invalid_scope_or_lease_releases_returned_lock_and_never_grants_authority() {
    for invalid in [
        body().replace("<d:exclusive/>", "<d:shared/>"),
        body().replace("<d:write/>", "<d:read/>"),
        body().replace("infinity", "0"),
        body().replace("Second-30", "Infinite"),
        body().replace("Second-30", "Second-0"),
        body().replace("Second-30", "Second-301"),
        body().replace("$ROOT", "https://elsewhere.test/inkstone/"),
        body().replace("$ROOT", ""),
        body().replace("19c78611", "00000000"),
        body().replace("<d:depth>infinity</d:depth>", ""),
        body().replace(
            "<d:depth>infinity</d:depth>",
            "<d:depth>infinity</d:depth><d:depth>infinity</d:depth>",
        ),
        "<broken".into(),
        "<!DOCTYPE d [<!ENTITY x 'bad'>]><d:prop xmlns:d='DAV:'>&x;</d:prop>".into(),
    ] {
        let (remote, task) = server(vec![grant(invalid), reply(204)]);
        assert!(remote.lock_maintenance().is_err());
        let requests = task.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("UNLOCK "));
    }
}

#[test]
fn unsupported_locked_and_multistatus_responses_do_not_grant_a_lock() {
    for code in [201, 207, 403, 405, 412, 423, 501] {
        let (remote, task) = server(vec![reply(code)]);
        let error = remote.lock_maintenance().err().unwrap().to_string();
        assert!(error.contains(&format!("HTTP {code}")));
        assert_eq!(task.join().unwrap().len(), 1);
    }
}

#[test]
fn failed_refresh_invalidates_guard_and_release_is_not_retried() -> Result<()> {
    let (remote, task) = server(vec![grant(body()), reply(412), reply(409)]);
    let mut lock = remote.lock_maintenance()?;
    assert!(lock.refresh().is_err());
    assert!(lock.condition().is_err());
    assert!(lock.refresh().is_err()); // No additional network request.
    assert!(lock.release().is_err()); // No hidden retry from Drop.
    assert_eq!(task.join().unwrap().len(), 3);
    Ok(())
}

#[test]
fn drop_releases_valid_guard_and_rejects_ambiguous_tokens() -> Result<()> {
    let (remote, task) = server(vec![grant(body()), reply(204)]);
    drop(remote.lock_maintenance()?);
    assert_eq!(task.join().unwrap().len(), 2);
    for token in [
        "",
        "urn:uuid:abc",
        "<relative>",
        "<DAV:no-lock>",
        "<urn:a> (<urn:b>)",
        "<urn:a b>",
    ] {
        assert!(token_uri(&HeaderValue::from_str(token)?).is_err());
    }
    for headers in [
        String::new(),
        format!("Lock-Token: {TOKEN}\r\nLock-Token: {TOKEN}\r\n"),
    ] {
        let (remote, task) = server(vec![(200, headers, body())]);
        assert!(remote.lock_maintenance().is_err());
        assert_eq!(task.join().unwrap().len(), 1);
    }
    Ok(())
}
