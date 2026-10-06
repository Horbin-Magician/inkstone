use super::*;
use std::net::TcpListener;

pub(super) const TOKEN: &str = "<urn:uuid:19c78611-2fb5-41ab-af92-4ca641921e95>";
pub(super) fn body() -> String {
    format!(
        r#"<d:prop xmlns:d="DAV:"><d:lockdiscovery><d:activelock><d:lockscope><d:exclusive/></d:lockscope><d:locktype><d:write/></d:locktype><d:depth>infinity</d:depth><d:locktoken><d:href>{}</d:href></d:locktoken><d:lockroot><d:href>$ROOT</d:href></d:lockroot><d:timeout>Second-30</d:timeout></d:activelock></d:lockdiscovery></d:prop>"#,
        &TOKEN[1..TOKEN.len() - 1]
    )
}
pub(super) fn grant(body: String) -> (u16, String, String) {
    (200, format!("Lock-Token: {TOKEN}\r\n"), body)
}
pub(super) fn reply(code: u16) -> (u16, String, String) {
    (code, String::new(), String::new())
}
pub(super) fn server(
    replies: Vec<(u16, String, String)>,
) -> (WebDav, std::thread::JoinHandle<Vec<String>>) {
    server_with_observer(replies, |_, _| {})
}

pub(super) fn server_with_observer(
    replies: Vec<(u16, String, String)>,
    mut observer: impl FnMut(usize, &str) + Send + 'static,
) -> (WebDav, std::thread::JoinHandle<Vec<String>>) {
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
            observer(requests.len(), &request);
            requests.push(request);
            let body = body.replace("$ROOT", &format!("{url}inkstone/"));
            write!(stream, "HTTP/1.1 {code} Test\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    (remote, task)
}

pub(super) fn manifest_reply(manifest: &Manifest, etag: &str) -> (u16, String, String) {
    (
        200,
        format!("ETag: \"{etag}\"\r\n"),
        serde_json::to_string(manifest).unwrap(),
    )
}
