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

fn preparation(manifest: &Manifest, objects: &BTreeMap<String, String>) -> (Manifest, Vec<Reply>) {
    let after = Manifest {
        version: 2,
        generation: Some(1),
        ..manifest.clone()
    };
    let mut replies = snapshot(manifest, objects, "before");
    replies.push(grant(body()));
    replies.extend(snapshot(manifest, objects, "before"));
    replies.extend(verification(objects));
    replies.extend(fence(manifest, &after));
    replies.extend(snapshot(&after, objects, "after"));
    replies.push(refresh());
    (after, replies)
}
fn candidates(manifest: &Manifest, objects: &BTreeMap<String, String>) -> Vec<(String, usize)> {
    objects
        .iter()
        .filter(|(digest, _)| {
            !manifest
                .files
                .values()
                .any(|referenced| referenced == *digest)
        })
        .map(|(digest, text)| (digest.clone(), text.len()))
        .collect()
}

#[test]
fn execute_deletes_only_verified_unreferenced_objects_and_confirms_absence() -> Result<()> {
    let (manifest, mut objects) = fixtures();
    objects.insert(hash(b"another old body"), "another old body".into());
    let candidates = candidates(&manifest, &objects);
    let (after, mut replies) = preparation(&manifest, &objects);
    replies.push(refresh());
    replies.extend(snapshot(&after, &objects, "after"));
    let mut remaining = objects.clone();
    for (digest, _) in &candidates {
        replies.extend([refresh(), reply(204), reply(404)]);
        remaining.remove(digest);
    }
    replies.push(refresh());
    replies.extend(snapshot(&after, &remaining, "after"));
    replies.push(reply(204));
    let (remote, task) = server(replies);
    let preview = remote.preview_cloud_cleanup()?;
    let checked = remote.prepare_cloud_cleanup(&preview, &Cancellation::default())?;
    let result = checked.execute(&Cancellation::default())?;
    assert_eq!(
        result,
        CloudCleanupReport {
            removed: 2,
            bytes: candidates.iter().map(|(_, bytes)| *bytes as u64).sum()
        }
    );
    let requests = task.join().unwrap();
    let deletes: Vec<_> = requests
        .iter()
        .filter(|request| request.starts_with("DELETE "))
        .collect();
    assert_eq!(deletes.len(), 2);
    for (request, (digest, _)) in deletes.iter().zip(&candidates) {
        assert!(request.starts_with(&format!(
            "DELETE /dav/inkstone/objects/{digest} HTTP/1.1\r\n"
        )));
        assert!(
            request
                .to_lowercase()
                .contains(&format!("if: <{}inkstone/> ({TOKEN})\r\n", remote.root))
        );
    }
    for digest in remaining.keys() {
        assert!(deletes.iter().all(|request| !request.contains(digest)));
    }
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("HEAD "))
            .count(),
        2
    );
    Ok(())
}

#[test]
fn execute_refuses_expired_lock_stale_plan_and_preexisting_cancellation() -> Result<()> {
    let (manifest, objects) = fixtures();
    for mode in 0..3 {
        let (after, mut replies) = preparation(&manifest, &objects);
        if mode == 0 {
            replies.push(reply(412));
        }
        if mode == 1 {
            let mut changed = objects.clone();
            changed.insert(hash(b"new upload"), "new upload".into());
            replies.push(refresh());
            replies.extend(snapshot(&after, &changed, "after"));
        }
        replies.push(reply(204));
        let (remote, task) = server(replies);
        let preview = remote.preview_cloud_cleanup()?;
        let cancellation = Cancellation::default();
        let checked = remote.prepare_cloud_cleanup(&preview, &cancellation)?;
        if mode == 2 {
            cancellation.request();
        }
        let error = checked.execute(&cancellation).unwrap_err();
        let failure = error.downcast_ref::<CloudCleanupFailure>().unwrap();
        assert_eq!(failure.completed.removed, 0);
        assert!(!failure.uncertain);
        assert!(!failure.lock_release_failed);
        assert_eq!(is_cloud_cleanup_cancelled(&error), mode == 2);
        assert!(
            task.join()
                .unwrap()
                .iter()
                .all(|request| !request.starts_with("DELETE "))
        );
    }
    Ok(())
}

#[test]
fn failed_or_unobserved_delete_preserves_confirmed_prefix_and_never_retries() -> Result<()> {
    let (manifest, mut objects) = fixtures();
    objects.insert(hash(b"another old body"), "another old body".into());
    let candidates = candidates(&manifest, &objects);
    for status in [0, 202, 207, 412, 423, 500] {
        let (after, mut replies) = preparation(&manifest, &objects);
        replies.push(refresh());
        replies.extend(snapshot(&after, &objects, "after"));
        replies.extend([
            refresh(),
            reply(204),
            reply(404),
            refresh(),
            reply(status),
            reply(204),
        ]);
        let (remote, task) = server(replies);
        let preview = remote.preview_cloud_cleanup()?;
        let checked = remote.prepare_cloud_cleanup(&preview, &Cancellation::default())?;
        let error = checked.execute(&Cancellation::default()).unwrap_err();
        let failure = error.downcast_ref::<CloudCleanupFailure>().unwrap();
        assert_eq!(
            failure.completed,
            CloudCleanupReport {
                removed: 1,
                bytes: candidates[0].1 as u64
            }
        );
        assert_eq!(failure.uncertain, !matches!(status, 412 | 423));
        assert!(!failure.cancelled);
        assert!(!failure.lock_release_failed);
        let requests = task.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("DELETE "))
                .count(),
            2
        );
        assert!(requests.last().unwrap().starts_with("UNLOCK "));
    }
    Ok(())
}

#[test]
fn cancellation_after_confirmed_delete_stops_remaining_candidates() -> Result<()> {
    use std::sync::Arc;
    let (manifest, mut objects) = fixtures();
    objects.insert(hash(b"another old body"), "another old body".into());
    let (after, mut replies) = preparation(&manifest, &objects);
    replies.push(refresh());
    replies.extend(snapshot(&after, &objects, "after"));
    replies.extend([refresh(), reply(204), reply(404), reply(204)]);
    let cancellation = Arc::new(Cancellation::default());
    let token = cancellation.clone();
    let (remote, task) = server_with_observer(replies, move |_, request| {
        if request.starts_with("HEAD ") {
            token.request();
        }
    });
    let preview = remote.preview_cloud_cleanup()?;
    let checked = remote.prepare_cloud_cleanup(&preview, &cancellation)?;
    let error = checked.execute(&cancellation).unwrap_err();
    let failure = error.downcast_ref::<CloudCleanupFailure>().unwrap();
    assert!(failure.cancelled);
    assert!(!failure.uncertain);
    assert_eq!(failure.completed.removed, 1);
    assert_eq!(
        task.join()
            .unwrap()
            .iter()
            .filter(|request| request.starts_with("DELETE "))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn absence_confirmation_and_lock_release_failures_are_reported_separately() -> Result<()> {
    let (manifest, objects) = fixtures();
    for absent in [false, true] {
        let (after, mut replies) = preparation(&manifest, &objects);
        replies.push(refresh());
        replies.extend(snapshot(&after, &objects, "after"));
        replies.extend([refresh(), reply(204), reply(if absent { 404 } else { 200 })]);
        if absent {
            let remaining = objects
                .iter()
                .filter(|(digest, _)| {
                    manifest
                        .files
                        .values()
                        .any(|reference| reference == *digest)
                })
                .map(|(digest, text)| (digest.clone(), text.clone()))
                .collect();
            replies.push(refresh());
            replies.extend(snapshot(&after, &remaining, "after"));
        }
        replies.push(reply(500));
        let (remote, task) = server(replies);
        let preview = remote.preview_cloud_cleanup()?;
        let checked = remote.prepare_cloud_cleanup(&preview, &Cancellation::default())?;
        let error = checked.execute(&Cancellation::default()).unwrap_err();
        let failure = error.downcast_ref::<CloudCleanupFailure>().unwrap();
        assert_eq!(failure.completed.removed, usize::from(absent));
        assert_eq!(failure.uncertain, !absent);
        assert!(failure.lock_release_failed);
        assert_eq!(
            task.join()
                .unwrap()
                .iter()
                .filter(|request| request.starts_with("UNLOCK "))
                .count(),
            1
        );
    }
    Ok(())
}
