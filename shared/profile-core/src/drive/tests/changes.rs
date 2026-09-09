use super::*;

#[tokio::test]
async fn change_watermarks_preserve_partial_pages_repetitions_and_unrelated_removals() {
    let directory = tempfile::tempdir().unwrap();
    let (worker, upload) = queued(&directory.path().join("history.sqlite")).await;
    let profile =
        json!({"fileId":FILE_ID,"removed":false,"changeType":"file","file":metadata(&upload)});
    let server = Server::start(vec![identity(), value(json!({"startPageToken":"first"})),
        value(json!({"changes":[profile.clone(),profile,
            {"fileId":"backup","removed":false,"changeType":"file","file":{"id":"backup","appProperties":{"shepBackup":"1"}}},
            {"fileId":"removed-file","removed":true,"changeType":"file"}
        ],"nextPageToken":"next + / ="})),
        value(json!({"changes":[],"newStartPageToken":"caught-up"})),
        value(json!({"changes":[],"newStartPageToken":"caught-up"})),
    ]).await;
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    let token = drive.start_page_token().await.unwrap();
    let page = drive.changes_page(&token).await.unwrap();
    assert_eq!(page.changes.len(), 4);
    assert!(matches!(&page.changes[0], FileChange::Profile(f) if *f == file(&upload)));
    assert!(matches!(&page.changes[1], FileChange::Profile(f) if *f == file(&upload)));
    assert!(matches!(&page.changes[2], FileChange::Other(id) if id == "backup"));
    assert!(matches!(&page.changes[3], FileChange::Removed(id) if id == "removed-file"));
    let ChangeCursor::More(token) = page.cursor else {
        panic!()
    };
    let page = drive.changes_page(&token).await.unwrap();
    assert!(page.changes.is_empty());
    assert_eq!(page.cursor, ChangeCursor::CaughtUp("caught-up".into()));
    assert_eq!(
        drive.changes_page("caught-up").await.unwrap().cursor,
        page.cursor
    );
    let requests = server.finish().await;
    assert_eq!(requests[1].url.path(), "/drive/v3/changes/startPageToken");
    let query: std::collections::HashMap<_, _> = requests[3].url.query_pairs().collect();
    assert_eq!(query["pageToken"], "next + / =");
    assert_eq!(query["spaces"], "appDataFolder");
    assert_eq!(query["includeRemoved"], "true");
    assert_eq!(query["restrictToMyDrive"], "false");
    assert_eq!(query["pageSize"], "50");
    worker.close().await.unwrap();
}

#[tokio::test]
async fn incomplete_or_invalid_change_pages_do_not_advance_a_checkpoint() {
    let invalid_entry =
        json!({"fileId":"file","removed":false,"changeType":"file","file":{"id":"another-file"}});
    for body in [
        json!({"changes":[]}),
        json!({"newStartPageToken":"valid"}),
        json!({"changes":[],"nextPageToken":"same"}),
        json!({"changes":[],"newStartPageToken":""}),
        json!({"changes":[],"nextPageToken":"next","newStartPageToken":"last"}),
        json!({"changes":[],"nextPageToken":null,"newStartPageToken":"last"}),
        json!({"changes":[invalid_entry],"newStartPageToken":"last"}),
        json!({"changes":[{"fileId":"file","removed":true,"changeType":"drive"}],"newStartPageToken":"last"}),
        json!({"changes":[{"fileId":"file","changeType":"file"}],"newStartPageToken":"last"}),
        json!({"changes":[{"fileId":"../file","removed":true,"changeType":"file"}],"newStartPageToken":"last"}),
        json!({"changes":vec![json!({"fileId":"file","removed":true,"changeType":"file"});51],"newStartPageToken":"last"}),
    ] {
        let server = Server::start(vec![identity(), value(body)]).await;
        let drive = server.connect(None).await.unwrap();
        assert!(drive.changes_page("same").await.is_err());
        assert!(drive.changes_page("bad\n").await.is_err());
        assert_eq!(server.finish().await.len(), 2);
    }
    for body in [
        json!({}),
        json!({"startPageToken":null}),
        json!({"startPageToken":""}),
    ] {
        let server = Server::start(vec![identity(), value(body)]).await;
        assert!(
            server
                .connect(None)
                .await
                .unwrap()
                .start_page_token()
                .await
                .is_err()
        );
        server.finish().await;
    }
}

#[tokio::test]
async fn changed_profile_markers_remain_observable_and_unowned_files_fail() {
    let directory = tempfile::tempdir().unwrap();
    let (worker, upload) = queued(&directory.path().join("history.sqlite")).await;
    let mut unowned = metadata(&upload);
    unowned["ownedByMe"] = json!(false);
    let server = Server::start(vec![identity(),
        value(json!({"changes":[{"fileId":FILE_ID,"removed":false,"changeType":"file","file":{"id":FILE_ID}}],"newStartPageToken":"one"})),
        value(json!({"changes":[{"fileId":FILE_ID,"removed":false,"changeType":"file","file":unowned}],"newStartPageToken":"two"})),
        reply(TestResponse::new(410, vec![])),
    ]).await;
    let drive = server.connect(None).await.unwrap();
    let page = drive.changes_page("start").await.unwrap();
    assert!(matches!(&page.changes[0], FileChange::Other(id) if id == FILE_ID));
    assert!(matches!(
        drive.changes_page("one").await,
        Err(Error::Invalid)
    ));
    assert!(matches!(
        drive.changes_page("one").await,
        Err(Error::Http(410))
    ));
    server.finish().await;
    worker.close().await.unwrap();
}
