use super::super::test_support::*;
use super::*;

type Reply = (u16, String, String);
fn fixtures() -> (Manifest, BTreeMap<String, String>) {
    let retained = "current content";
    let candidate = "old content";
    (
        Manifest {
            files: BTreeMap::from([
                ("one.md".into(), hash(retained.as_bytes())),
                ("duplicate.md".into(), hash(retained.as_bytes())),
            ]),
            ..Manifest::default()
        },
        BTreeMap::from([
            (hash(retained.as_bytes()), retained.into()),
            (hash(candidate.as_bytes()), candidate.into()),
        ]),
    )
}
fn listing(objects: &BTreeMap<String, String>) -> String {
    let rows: String = objects.iter().map(|(digest, text)| format!("<d:response><d:href>{digest}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{}</d:getcontentlength></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>", text.len())).collect();
    format!("<d:multistatus xmlns:d=\"DAV:\">{rows}</d:multistatus>")
}
fn snapshot(manifest: &Manifest, objects: &BTreeMap<String, String>, etag: &str) -> Vec<Reply> {
    vec![
        reply(404),
        manifest_reply(manifest, etag),
        (207, String::new(), listing(objects)),
        reply(404),
        manifest_reply(manifest, etag),
    ]
}
fn refresh() -> Reply {
    (200, String::new(), body())
}
fn verification(objects: &BTreeMap<String, String>) -> Vec<Reply> {
    objects
        .values()
        .flat_map(|text| [refresh(), (200, String::new(), text.clone())])
        .collect()
}
fn fence(before: &Manifest, after: &Manifest) -> Vec<Reply> {
    vec![
        refresh(),
        reply(404),
        manifest_reply(before, "before"),
        reply(204),
        reply(404),
        manifest_reply(after, "after"),
    ]
}

#[test]
fn preview_is_read_only_and_checked_plan_verifies_all_bodies_before_fencing() -> Result<()> {
    let (manifest, objects) = fixtures();
    let after = Manifest {
        version: 2,
        generation: Some(1),
        ..manifest.clone()
    };
    let mut responses = snapshot(&manifest, &objects, "before");
    responses.push(grant(body()));
    responses.extend(snapshot(&manifest, &objects, "before"));
    responses.extend(verification(&objects));
    responses.extend(fence(&manifest, &after));
    responses.extend(snapshot(&after, &objects, "after"));
    responses.extend([refresh(), reply(204)]);
    let (remote, task) = server(responses);
    let preview = remote.preview_cloud_cleanup()?;
    assert_eq!(
        (
            preview.candidates,
            preview.candidate_bytes,
            preview.retained,
            preview.retained_bytes
        ),
        (1, 11, 1, 15)
    );
    let checked = remote.prepare_cloud_cleanup(&preview, &Cancellation::default())?;
    assert_eq!(checked.preview().manifest, after);
    assert_eq!(checked.preview().objects, preview.objects);
    checked.release()?;
    let requests = task.join().unwrap();
    assert!(
        requests[..5]
            .iter()
            .all(|r| r.starts_with("GET ") || r.starts_with("PROPFIND "))
    );
    let put = requests.iter().position(|r| r.starts_with("PUT ")).unwrap();
    for digest in objects.keys() {
        let get = requests
            .iter()
            .position(|r| r.starts_with(&format!("GET /dav/inkstone/objects/{digest} ")))
            .unwrap();
        assert!(get < put);
    }
    assert!(requests.iter().all(|r| !r.starts_with("DELETE ")));
    Ok(())
}

#[test]
fn stale_preview_and_wrong_endpoint_cannot_publish_a_fence() -> Result<()> {
    let (manifest, objects) = fixtures();
    let mut changed = objects.clone();
    changed.insert(hash(b"another"), "another".into());
    let mut replies = snapshot(&manifest, &objects, "before");
    replies.push(grant(body()));
    replies.extend(snapshot(&manifest, &changed, "before"));
    replies.push(reply(204));
    let (remote, task) = server(replies);
    let preview = remote.preview_cloud_cleanup()?;
    let other = WebDav::new(
        &Settings {
            url: "http://127.0.0.1:1/".into(),
            ..Default::default()
        },
        "",
    )?;
    assert!(
        other
            .prepare_cloud_cleanup(&preview, &Cancellation::default())
            .err()
            .unwrap()
            .to_string()
            .contains("其他服务器")
    );
    assert!(
        remote
            .prepare_cloud_cleanup(&preview, &Cancellation::default())
            .err()
            .unwrap()
            .to_string()
            .contains("重新预览")
    );
    assert!(task.join().unwrap().iter().all(|r| !r.starts_with("PUT ")));
    Ok(())
}

#[test]
fn corrupt_retained_or_candidate_body_stops_before_manifest_mutation() -> Result<()> {
    let (manifest, objects) = fixtures();
    for corrupt in objects.keys() {
        let mut responses = snapshot(&manifest, &objects, "before");
        responses.push(grant(body()));
        responses.extend(snapshot(&manifest, &objects, "before"));
        for (digest, text) in &objects {
            responses.push(refresh());
            responses.push((
                200,
                String::new(),
                if digest == corrupt {
                    "X".repeat(text.len())
                } else {
                    text.clone()
                },
            ));
            if digest == corrupt {
                break;
            }
        }
        responses.push(reply(204));
        let (remote, task) = server(responses);
        let preview = remote.preview_cloud_cleanup()?;
        assert!(
            remote
                .prepare_cloud_cleanup(&preview, &Cancellation::default())
                .err()
                .unwrap()
                .to_string()
                .contains("校验失败")
        );
        assert!(
            task.join()
                .unwrap()
                .iter()
                .all(|r| !r.starts_with("PUT ") && !r.starts_with("DELETE "))
        );
    }
    Ok(())
}

#[test]
fn cancellation_during_verification_or_after_fence_preserves_objects_and_releases_lock()
-> Result<()> {
    use std::sync::Arc;
    let (manifest, objects) = fixtures();
    let after = Manifest {
        version: 2,
        generation: Some(1),
        ..manifest.clone()
    };
    for after_fence in [false, true] {
        let mut responses = snapshot(&manifest, &objects, "before");
        responses.push(grant(body()));
        responses.extend(snapshot(&manifest, &objects, "before"));
        if after_fence {
            responses.extend(verification(&objects));
            responses.extend(fence(&manifest, &after));
        } else {
            responses.extend(verification(&objects).into_iter().take(2));
        }
        responses.push(reply(204));
        let cancellation = Arc::new(Cancellation::default());
        let token = cancellation.clone();
        let (remote, task) = server_with_observer(responses, move |_, request| {
            if (after_fence && request.starts_with("PUT "))
                || (!after_fence && request.starts_with("GET /dav/inkstone/objects/"))
            {
                token.request();
            }
        });
        let preview = remote.preview_cloud_cleanup()?;
        let error = remote
            .prepare_cloud_cleanup(&preview, &cancellation)
            .err()
            .unwrap();
        assert!(error.is::<Cancelled>());
        let requests = task.join().unwrap();
        assert!(requests.last().unwrap().starts_with("UNLOCK "));
        assert!(requests.iter().all(|r| !r.starts_with("DELETE ")));
        assert_eq!(
            requests.iter().filter(|r| r.starts_with("PUT ")).count(),
            usize::from(after_fence)
        );
    }
    Ok(())
}

#[test]
fn incomplete_listing_missing_references_and_unknown_types_are_not_cleanup_previews() -> Result<()>
{
    let (manifest, objects) = fixtures();
    let root = Url::parse("https://example.test/inkstone/objects/")?;
    for xml in [
        listing(&BTreeMap::new()),
        listing(&objects).replace(
            "<d:resourcetype/>",
            "<d:resourcetype><x:unknown xmlns:x=\"urn:extension\"/></d:resourcetype>",
        ),
        listing(&objects).replace("<d:resourcetype/>", "<d:resourcetype/><d:resourcetype/>"),
        listing(&objects).replace(
            "<d:propstat>",
            "<d:status>HTTP/1.1 404 Not Found</d:status><d:propstat>",
        ),
        listing(&objects).replace(
            "<d:status>HTTP/1.1 200 OK</d:status>",
            "<d:status>HTTP/1.1 200 OK</d:status><d:status>HTTP/1.1 404 Not Found</d:status>",
        ),
        listing(&objects).replace("</d:multistatus>", "<d:unknown/></d:multistatus>"),
    ] {
        let listing = capacity::parse_objects(&root, xml.as_bytes())?;
        assert!(
            preview(
                "identity".into(),
                manifest.clone(),
                Some("revision".into()),
                listing
            )
            .is_err()
        );
    }
    Ok(())
}
