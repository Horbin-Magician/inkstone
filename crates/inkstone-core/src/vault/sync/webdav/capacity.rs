//! Read-only object accounting. Unreferenced objects are NOT cleanup candidates:
//! another client may have uploaded them without publishing its manifest yet.
use super::*;

const MAX_LISTING_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloudCapacity {
    pub referenced_objects: usize,
    pub referenced_bytes: u64,
    pub unreferenced_objects: usize,
    pub unreferenced_bytes: u64,
    /// Manifest objects absent from the listing, or with unavailable length.
    pub missing_or_unknown_objects: usize,
    /// Non-object responses and unavailable/malformed properties, excluding the root collection.
    pub unrecognized_entries: usize,
}

impl WebDav {
    /// Snapshot of server-reported lengths, not hash verification or reclaimable space.
    /// No collection creation, object download, publication or deletion is performed.
    pub fn capacity(&self) -> Result<CloudCapacity> {
        let (before, revision) = self.manifest()?;
        let listing = self.object_listing()?;
        let report = summarize(&before, listing.objects, listing.unrecognized)?;
        let (after, current_revision) = self.manifest()?;
        ensure!(
            revision == current_revision && before == after,
            "统计期间云端清单发生变化，请重新刷新容量"
        );
        Ok(report)
    }
}

#[derive(Default)]
pub(super) struct Listing {
    pub objects: BTreeMap<String, u64>,
    pub unrecognized: usize,
}

impl WebDav {
    pub(super) fn object_listing(&self) -> Result<Listing> {
        let response = self.send(
            self.request(Method::from_bytes(b"PROPFIND")?, "inkstone/objects/")
                .header("Depth", "1")
                .header(header::CONTENT_TYPE, "application/xml")
                .header(header::CACHE_CONTROL, "no-cache")
                .body("<?xml version=\"1.0\"?><d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/><d:getcontentlength/></d:prop></d:propfind>"),
        )?;
        if response.status() == StatusCode::NOT_FOUND {
            Ok(Listing::default())
        } else {
            let response = Self::status(response)?;
            ensure!(
                response.status() == StatusCode::MULTI_STATUS,
                "服务器未返回 WebDAV 对象列表"
            );
            let bytes = Self::body(response, MAX_LISTING_BYTES)?;
            parse_objects(&self.root.join("inkstone/objects/")?, &bytes)
        }
    }
}

fn summarize(
    manifest: &Manifest,
    mut objects: BTreeMap<String, u64>,
    unrecognized: usize,
) -> Result<CloudCapacity> {
    let mut report = CloudCapacity {
        unrecognized_entries: unrecognized,
        ..Default::default()
    };
    // Identical content shared by multiple notes occupies only one remote object.
    for digest in manifest.files.values().collect::<BTreeSet<_>>() {
        if let Some(bytes) = objects.remove(digest) {
            report.referenced_objects += 1;
            report.referenced_bytes = report
                .referenced_bytes
                .checked_add(bytes)
                .context("云端容量超出可统计范围")?;
        } else {
            report.missing_or_unknown_objects += 1;
        }
    }
    report.unreferenced_objects = objects.len();
    for bytes in objects.into_values() {
        report.unreferenced_bytes = report
            .unreferenced_bytes
            .checked_add(bytes)
            .context("云端容量超出可统计范围")?;
    }
    Ok(report)
}

#[cfg(test)]
fn parse_listing(directory: &Url, manifest: &Manifest, bytes: &[u8]) -> Result<CloudCapacity> {
    let listing = parse_objects(directory, bytes)?;
    summarize(manifest, listing.objects, listing.unrecognized)
}

pub(super) fn parse_objects(directory: &Url, bytes: &[u8]) -> Result<Listing> {
    let text = std::str::from_utf8(bytes).context("WebDAV 对象列表不是 UTF-8")?;
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 1_000_000,
        },
    )
    .context("WebDAV 对象列表格式无效或超过限制")?;
    let root = document.root_element();
    ensure!(
        root.has_tag_name(("DAV:", "multistatus")),
        "WebDAV 对象列表缺少 multistatus"
    );
    let mut objects = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut unknown = 0;
    for response in root.children().filter(|n| n.is_element()) {
        if !response.has_tag_name(("DAV:", "response")) {
            unknown += 1;
            continue;
        }
        let hrefs: Vec<_> = response
            .children()
            .filter(|n| n.has_tag_name(("DAV:", "href")))
            .collect();
        let Some(url) = hrefs
            .first()
            .filter(|_| hrefs.len() == 1)
            .and_then(|n| n.text())
            .and_then(|s| directory.join(s.trim()).ok())
        else {
            unknown += 1;
            continue;
        };
        if url == *directory {
            continue;
        }
        let digest = url.path().strip_prefix(directory.path()).unwrap_or("");
        if url.origin() != directory.origin()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || !valid_hash(digest)
        {
            unknown += 1;
            continue;
        }
        ensure!(
            seen.insert(digest.to_owned()),
            "WebDAV 对象列表包含重复记录"
        );
        let mut length = None;
        let mut collection = false;
        let mut type_known = false;
        let mut invalid_length = response
            .children()
            .any(|n| n.has_tag_name(("DAV:", "status")));
        for propstat in response
            .children()
            .filter(|n| n.has_tag_name(("DAV:", "propstat")))
        {
            let mut statuses = propstat
                .children()
                .filter(|n| n.has_tag_name(("DAV:", "status")));
            let status = statuses.next().and_then(|n| n.text());
            if status.and_then(|s| s.split_whitespace().nth(1)) != Some("200")
                || statuses.next().is_some()
            {
                invalid_length = true;
                continue;
            }
            for prop in propstat
                .children()
                .filter(|n| n.has_tag_name(("DAV:", "prop")))
            {
                for property in prop.children().filter(|n| n.is_element()) {
                    if property.has_tag_name(("DAV:", "getcontentlength")) {
                        let parsed = property.text().and_then(|s| s.trim().parse::<u64>().ok());
                        if parsed.is_none() || length.is_some() {
                            invalid_length = true;
                        }
                        length = parsed;
                    } else if property.has_tag_name(("DAV:", "resourcetype")) {
                        invalid_length |= type_known;
                        type_known = true;
                        // Only an empty resource type denotes an ordinary object.
                        // Unknown DAV extension types must never become cleanup candidates.
                        collection |= property.children().any(|n| n.is_element())
                            || property.text().is_some_and(|s| !s.trim().is_empty());
                    }
                }
            }
        }
        if let Some(bytes) = length.filter(|_| !invalid_length && type_known && !collection) {
            objects.insert(digest.to_owned(), bytes);
        } else {
            unknown += 1;
        }
    }
    Ok(Listing {
        objects,
        unrecognized: unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(href: &str, length: &str, status: &str) -> String {
        format!(
            "<d:response><d:href>{href}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{length}</d:getcontentlength></d:prop><d:status>HTTP/1.1 {status}</d:status></d:propstat></d:response>"
        )
    }
    fn listing(rows: &str) -> String {
        format!("<d:multistatus xmlns:d=\"DAV:\">{rows}</d:multistatus>")
    }
    fn manifest() -> Manifest {
        Manifest {
            version: 1,
            files: BTreeMap::from([
                ("one.md".into(), "a".repeat(64)),
                ("same.md".into(), "a".repeat(64)),
                ("missing.md".into(), "c".repeat(64)),
            ]),
            ..Manifest::default()
        }
    }
    #[test]
    fn listing_counts_unique_references_without_calling_unreferenced_data_reclaimable() {
        let directory = Url::parse("https://example.test/notes/inkstone/objects/").unwrap();
        let rows = response(&"a".repeat(64), "123", "200 OK")
            + &response(
                &format!("/notes/inkstone/objects/{}", "b".repeat(64)),
                "456",
                "200 OK",
            )
            + &response(&"c".repeat(64), "0", "404 Not Found")
            + &response(
                "https://other.test/notes/inkstone/objects/unrelated",
                "999",
                "200 OK",
            )
            + &response("../manifest.json", "999", "200 OK")
            + &response("/notes/inkstone/objects/", "0", "200 OK");
        let report = parse_listing(&directory, &manifest(), listing(&rows).as_bytes()).unwrap();
        assert_eq!(
            report,
            CloudCapacity {
                referenced_objects: 1,
                referenced_bytes: 123,
                unreferenced_objects: 1,
                unreferenced_bytes: 456,
                missing_or_unknown_objects: 1,
                unrecognized_entries: 3,
            }
        );
    }
    #[test]
    fn malformed_properties_duplicates_overflow_and_xml_are_not_reported_as_valid_capacity() {
        let directory = Url::parse("https://example.test/inkstone/objects/").unwrap();
        let a = "a".repeat(64);
        for row in [
            response(&a, "not-a-size", "200 OK"),
            response(&a, "-1", "200 OK"),
            response(&a, "42", "403 Forbidden"),
            response(&a, "42", "200 OK").replace("<d:resourcetype/>", ""),
            response(&a, "42", "200 OK").replace(
                "<d:resourcetype/>",
                "<d:resourcetype><d:collection/></d:resourcetype>",
            ),
        ] {
            let r = parse_listing(&directory, &manifest(), listing(&row).as_bytes()).unwrap();
            assert_eq!(
                (
                    r.referenced_objects,
                    r.referenced_bytes,
                    r.missing_or_unknown_objects,
                    r.unrecognized_entries
                ),
                (0, 0, 2, 1)
            );
        }
        let row = response(&a, "42", "200 OK");
        assert!(
            parse_listing(
                &directory,
                &manifest(),
                listing(&(row.clone() + &row)).as_bytes()
            )
            .is_err()
        );
        let overflow = response(&a, &u64::MAX.to_string(), "200 OK")
            + &response(&"b".repeat(64), "1", "200 OK");
        assert!(
            parse_listing(
                &directory,
                &Manifest::default(),
                listing(&overflow).as_bytes()
            )
            .is_err()
        );
        for xml in [
            "<broken",
            "<multistatus/>",
            "<!DOCTYPE d [<!ENTITY x 'bad'>]><d:multistatus xmlns:d='DAV:'>&x;</d:multistatus>",
        ] {
            assert!(parse_listing(&directory, &manifest(), xml.as_bytes()).is_err());
        }
    }
    #[test]
    fn capacity_http_is_read_only_and_rejects_manifest_changes() {
        use std::net::TcpListener;
        for changed in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let manifest = serde_json::to_string(&manifest()).unwrap();
                let xml = listing(&response(&"a".repeat(64), "123", "200 OK"));
                for (index, (method, path)) in [
                    ("GET", "/inkstone-v1/manifest.json"),
                    ("GET", "/inkstone/manifest.json"),
                    ("PROPFIND", "/inkstone/objects/"),
                    ("GET", "/inkstone-v1/manifest.json"),
                    ("GET", "/inkstone/manifest.json"),
                ]
                .into_iter()
                .enumerate()
                {
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    let (mut stream, _) = loop {
                        match listener.accept() {
                            Ok(connection) => break connection,
                            Err(error)
                                if error.kind() == io::ErrorKind::WouldBlock
                                    && std::time::Instant::now() < deadline =>
                            {
                                std::thread::sleep(Duration::from_millis(1))
                            }
                            Err(error) => panic!("missing request {index}: {error}"),
                        }
                    };
                    // Accepted sockets can inherit the listener's nonblocking mode on macOS.
                    // Poll only accept; request reads use the bounded blocking timeout below.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        header.push(byte[0]);
                    }
                    let text = String::from_utf8(header).unwrap();
                    let length = text
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).unwrap();
                    assert!(
                        text.starts_with(&format!("{method} {path} HTTP/1.1\r\n")),
                        "{text}"
                    );
                    let (status, body) = if path.contains("-v1") {
                        ("404 Not Found", "")
                    } else if method == "PROPFIND" {
                        assert!(text.to_lowercase().contains("depth: 1\r\n"));
                        ("207 Multi-Status", xml.as_str())
                    } else {
                        ("200 OK", manifest.as_str())
                    };
                    let etag = if changed && index == 4 {
                        "changed"
                    } else {
                        "stable"
                    };
                    write!(stream, "HTTP/1.1 {status}\r\nETag: \"{etag}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            });
            let remote = WebDav::new(
                &Settings {
                    url: format!("http://{address}/"),
                    ..Default::default()
                },
                "",
            )
            .unwrap();
            let result = remote.capacity();
            server.join().unwrap();
            if changed {
                assert!(result.unwrap_err().to_string().contains("清单发生变化"));
            } else {
                assert_eq!(result.unwrap().referenced_bytes, 123);
            }
        }
    }
}
