use super::super::test_support::*;
use super::*;

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

#[test]
fn maintenance_advances_manifest_under_both_lock_and_revision_conditions() -> Result<()> {
    for generation in [None, Some(7)] {
        let before = Manifest {
            version: if generation.is_some() { 2 } else { 1 },
            generation,
            files: BTreeMap::from([("note.md".into(), hash(b"unchanged body"))]),
            ..Manifest::default()
        };
        let after = Manifest {
            version: 2,
            generation: Some(generation.unwrap_or(0) + 1),
            ..before.clone()
        };
        let (remote, task) = server(vec![
            grant(body()),
            reply(404),
            manifest_reply(&before, "before"),
            reply(204),
            reply(404),
            manifest_reply(&after, "after"),
            reply(204),
        ]);
        let mut lock = remote.lock_maintenance()?;
        let (confirmed, revision) = lock.advance_manifest()?;
        assert_eq!(confirmed, after);
        assert_eq!(revision, "\"after\"");
        lock.release()?;
        let requests = task.join().unwrap();
        assert_eq!(requests.len(), 7);
        let publish = requests[3].to_lowercase();
        assert!(publish.starts_with("put /dav/inkstone/manifest.json http/1.1\r\n"));
        assert!(publish.contains("if-match: \"before\"\r\n"));
        assert!(publish.contains(&format!("if: <{}inkstone/> ({TOKEN})\r\n", remote.root)));
        let published: Manifest =
            serde_json::from_str(requests[3].split_once("\r\n\r\n").unwrap().1)?;
        assert_eq!(published, after);
    }
    Ok(())
}

#[test]
fn maintenance_rejects_failed_publication_readback_and_generation_overflow() -> Result<()> {
    let before = Manifest::default();
    let after = Manifest {
        version: 2,
        generation: Some(1),
        ..before.clone()
    };
    for (confirmed, etag) in [(before.clone(), "after"), (after.clone(), "before")] {
        let (remote, task) = server(vec![
            grant(body()),
            reply(404),
            manifest_reply(&before, "before"),
            reply(204),
            reply(404),
            manifest_reply(&confirmed, etag),
            reply(204),
        ]);
        let mut lock = remote.lock_maintenance()?;
        assert!(lock.advance_manifest().is_err());
        assert!(lock.condition().is_err());
        assert!(lock.advance_manifest().is_err());
        lock.release()?;
        let requests = task.join().unwrap();
        assert_eq!(requests.iter().filter(|r| r.starts_with("PUT ")).count(), 1); // No rollback.
    }
    for status in [207, 412, 423, 500] {
        let (remote, task) = server(vec![
            grant(body()),
            reply(404),
            manifest_reply(&before, "before"),
            reply(status),
            reply(204),
        ]);
        let mut lock = remote.lock_maintenance()?;
        assert!(lock.advance_manifest().is_err());
        assert!(lock.condition().is_err());
        lock.release()?;
        assert_eq!(task.join().unwrap().len(), 5);
    }
    let overflow = Manifest {
        version: 2,
        generation: Some(u64::MAX),
        ..before
    };
    let (remote, task) = server(vec![
        grant(body()),
        reply(404),
        manifest_reply(&overflow, "before"),
        reply(204),
    ]);
    let mut lock = remote.lock_maintenance()?;
    assert!(
        lock.advance_manifest()
            .unwrap_err()
            .to_string()
            .contains("代次已耗尽")
    );
    lock.release()?;
    assert!(task.join().unwrap().iter().all(|r| !r.starts_with("PUT ")));
    Ok(())
}

#[test]
fn first_manifest_is_fenced_with_create_only_precondition() -> Result<()> {
    let after = Manifest {
        version: 2,
        generation: Some(1),
        ..Manifest::default()
    };
    let (remote, task) = server(vec![
        grant(body()),
        reply(404),
        reply(404),
        reply(201),
        reply(404),
        manifest_reply(&after, "created"),
        reply(204),
    ]);
    let mut lock = remote.lock_maintenance()?;
    assert_eq!(lock.advance_manifest()?.0, after);
    lock.release()?;
    let requests = task.join().unwrap();
    assert!(requests[3].to_lowercase().contains("if-none-match: *\r\n"));
    Ok(())
}
