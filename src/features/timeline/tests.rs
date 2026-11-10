use super::*;
use megalodon::entities::Notification;
use serde_json::json;
fn post(id: &str) -> Status {
    let mut post = notification(id, "mention", "alice", "2026-01-01T12:00:00Z", true)
        .status
        .unwrap();
    post.id = id.into();
    post
}
fn feed() -> Timeline {
    Timeline::new(
        Client::new("https://example.org".into(), None),
        TimelineKind::Public,
    )
}
fn page(ids: &[&str], cursor: &str, more: bool) -> fetch::Page {
    fetch::Page {
        statuses: ids.iter().map(|id| post(id)).collect(),
        cursor: Some(cursor.into()),
        more,
    }
}
#[test]
fn failures_preserve_posts_and_retry_the_same_page_with_a_new_request() {
    let mut feed = feed();
    let _ = feed.update(Message::PageLoaded(0, Ok(page(&["1"], "cursor", true))));
    let _ = feed.update(Message::LoadMore);
    let failed = feed.request;
    let _ = feed.update(Message::PageLoaded(failed, Err("offline".into())));
    assert_eq!(feed.statuses, ["1".to_string()]);
    assert_eq!(feed.max_id.as_deref(), Some("cursor"));
    assert!(!feed.loading);
    let _ = feed.update(Message::Retry);
    assert!(feed.loading && feed.older);
    assert!(feed.request > failed);
    assert!(feed.error.is_none());
}
#[test]
fn refresh_restarts_pagination_and_stale_pages_cannot_replace_it() {
    let mut feed = feed();
    let _ = feed.update(Message::PageLoaded(0, Ok(page(&["1"], "cursor", true))));
    let _ = feed.update(Message::Refresh);
    let _ = feed.update(Message::PageLoaded(0, Ok(page(&["stale"], "old", false))));
    assert_eq!(feed.statuses, ["1".to_string()]);
    let _ = feed.update(Message::PageLoaded(
        feed.request,
        Ok(page(&["2"], "new-cursor", true)),
    ));
    assert_eq!(feed.statuses, ["2".to_string()]);
    assert_eq!(feed.max_id.as_deref(), Some("new-cursor"));
    let _ = feed.update(Message::LoadMore);
    let _ = feed.update(Message::PageLoaded(
        feed.request,
        Ok(page(&["2", "3"], "older-cursor", false)),
    ));
    assert_eq!(feed.statuses, ["2".to_string(), "3".to_string()]);
    assert!(!feed.more);
}
#[test]
fn stream_posts_wait_for_publish_but_own_posts_are_immediate_and_deletions_remove_both() {
    let mut feed = feed();
    let _ = feed.update(Message::IncomingStatus(post("1")));
    let _ = feed.update(Message::IncomingStatus(post("1")));
    assert!(feed.statuses.is_empty());
    assert_eq!(feed.incoming.len(), 1);
    let _ = feed.update(Message::PrependStatus(post("1")));
    assert!(feed.incoming.is_empty());
    assert_eq!(feed.statuses, ["1".to_string()]);
    let _ = feed.update(Message::IncomingStatus(post("2")));
    let _ = feed.update(Message::PublishNew);
    assert_eq!(feed.statuses, ["2".to_string(), "1".to_string()]);
    let _ = feed.update(Message::IncomingStatus(post("3")));
    let _ = feed.update(Message::DeleteStatus("3".into()));
    let _ = feed.update(Message::DeleteStatus("2".into()));
    assert!(feed.incoming.is_empty());
    assert_eq!(feed.statuses, ["1".to_string()]);
}
#[test]
fn account_reset_clears_buffered_posts_errors_and_invalidates_previous_requests() {
    let mut feed = feed();
    let _ = feed.update(Message::IncomingStatus(post("1")));
    let previous = feed.request;
    feed.reset(Client::new("https://another.example.org".into(), None));
    let _ = feed.update(Message::PageLoaded(
        previous,
        Ok(page(&["1"], "cursor", true)),
    ));
    assert!(feed.statuses.is_empty() && feed.incoming.is_empty());
    assert!(!feed.has_loaded && feed.error.is_none());
}
#[test]
fn repeated_pagination_cursor_stops_offering_duplicate_pages() {
    let mut feed = feed();
    let _ = feed.update(Message::PageLoaded(0, Ok(page(&["1"], "cursor", true))));
    let _ = feed.update(Message::LoadMore);
    let _ = feed.update(Message::PageLoaded(
        feed.request,
        Ok(page(&["1"], "cursor", true)),
    ));
    assert!(!feed.more);
    assert_eq!(feed.statuses.len(), 1);
}
fn notification(id: &str, kind: &str, actor: &str, date: &str, post: bool) -> Notification {
    let account = json!({
        "id":actor,"username":actor,"acct":actor,"display_name":"","locked":false,
        "created_at":"2020-01-01T00:00:00Z","followers_count":0,"following_count":0,
        "statuses_count":0,"note":"","url":"https://example.org/@alice","avatar":"",
        "avatar_static":"","header":"","header_static":"","emojis":[],"fields":[],"bot":false
    });
    let status = post.then(|| json!({
        "id":"s1","uri":"https://example.org/posts/s1","account":account,"content":"<p>Hello</p>",
        "created_at":"2020-01-01T00:00:00Z","emojis":[],"replies_count":0,"reblogs_count":0,
        "favourites_count":0,"sensitive":false,"spoiler_text":"","visibility":"public",
        "media_attachments":[],"mentions":[],"tags":[],
        "quote_approval":{"automatic":[],"manual":[],"current_user":"unknown"}
    }));
    serde_json::from_value(
        json!({"id":id,"type":kind,"account":account,"created_at":date,"status":status}),
    )
    .unwrap()
}

#[test]
fn a_late_page_cannot_restore_a_stream_deleted_post() {
    let mut feed = feed();
    let _ = feed.update(Message::IncomingStatus(post("1")));
    let _ = feed.update(Message::DeleteStatus("1".into()));
    let _ = feed.update(Message::PageLoaded(
        0,
        Ok(page(&["1", "2"], "cursor", true)),
    ));
    let _ = feed.update(Message::IncomingStatus(post("1")));
    assert_eq!(feed.statuses, ["2".to_string()]);
    assert!(feed.incoming.is_empty());
}

#[test]
fn edits_update_boosts_and_notifications_and_survive_late_rest_responses() {
    let mut cache = Cache::new();
    let original = post("1");
    let mut boost = post("boost");
    boost.reblog = Some(Box::new(original.clone()));
    cache.insert_status(boost.clone());
    let mut activity = notification("n1", "mention", "alice", "2026-01-01T12:00:00Z", true);
    activity.status = Some(original.clone());
    cache.insert_notification(activity.clone());
    let mut edited = original.clone();
    edited.content = "<p>Edited</p>".into();
    edited.edited_at = Some(chrono::Utc::now());
    cache.insert_status(edited.clone());
    cache.insert_status(boost); // old embedded copy must not replace the edit
    cache.insert_notification(activity);
    cache.insert_status(original);
    assert_eq!(cache.statuses["1"].content, edited.content);
    assert_eq!(
        cache.statuses["boost"].reblog.as_ref().unwrap().content,
        edited.content
    );
    assert_eq!(
        cache.notifications["n1"].status.as_ref().unwrap().content,
        edited.content
    );
}

#[test]
fn deletions_remove_boosts_and_notifications_and_reject_late_copies() {
    let mut cache = Cache::new();
    let original = post("1");
    let mut boost = post("boost");
    boost.reblog = Some(Box::new(original.clone()));
    let mut activity = notification("n1", "mention", "alice", "2026-01-01T12:00:00Z", true);
    activity.status = Some(original.clone());
    cache.insert_status(boost.clone());
    cache.insert_notification(activity.clone());
    let removed = cache.delete_status("1");
    assert!(removed.contains(&"boost".to_owned()));
    cache.insert_status(original);
    cache.insert_status(boost);
    cache.insert_notification(activity);
    assert!(cache.statuses.is_empty());
    assert!(cache.notifications.is_empty());
    cache.clear();
    // Tombstones belong to one account, not the next account on the same server.
    cache.insert_status(post("1"));
    assert!(cache.statuses.contains_key("1"));
}

#[test]
fn reconnect_supersedes_an_inflight_page_without_discarding_visible_posts() {
    let mut feed = feed();
    let _ = feed.update(Message::PageLoaded(0, Ok(page(&["1"], "old", true))));
    let _ = feed.update(Message::LoadMore);
    let old_request = feed.request;
    let _ = feed.update(Message::Recover);
    assert!(feed.loading && !feed.older);
    assert_eq!(feed.statuses, ["1".to_owned()]);
    let _ = feed.update(Message::PageLoaded(
        old_request,
        Ok(page(&["stale"], "old", false)),
    ));
    assert!(feed.loading);
    let _ = feed.update(Message::PageLoaded(
        feed.request,
        Ok(page(&["new", "1"], "next", true)),
    ));
    assert_eq!(feed.statuses, ["new".to_owned(), "1".to_owned()]);
    assert_eq!(feed.max_id.as_deref(), Some("next"));
}

#[test]
fn saved_collection_changes_remove_rows_and_supersede_stale_pages() {
    for kind in [TimelineKind::Favorites, TimelineKind::Bookmarks] {
        let mut feed = Timeline::new(
            Client::new("https://example.org".into(), Some("token".into())),
            kind,
        );
        let _ = feed.update(Message::PageLoaded(0, Ok(page(&["1", "2"], "2", true))));
        let _ = feed.update(Message::Refresh);
        let stale = feed.request;
        let _ = feed.update(Message::CollectionChanged(post("1")));
        assert_eq!(feed.statuses, ["2".to_string()]);
        assert!(!feed.loading && feed.has_loaded);
        assert!(cosmic::iced::advanced::subscription::into_recipes(feed.subscription()).is_empty());
        let _ = feed.update(Message::PageLoaded(stale, Ok(page(&["1"], "1", false))));
        assert_eq!(feed.statuses, ["2".to_string()]);
        let mut added = post("3");
        added.favourited = Some(true);
        added.bookmarked = Some(true);
        let _ = feed.update(Message::CollectionChanged(added));
        assert_eq!(feed.statuses, ["3".to_string(), "2".to_string()]);
        assert!(!feed.loading);
        assert_eq!(feed.max_id.as_deref(), Some("2"));
    }
}

#[test]
fn like_unlike_and_relike_keep_counts_and_embedded_copies_in_sync() {
    let mut cache = Cache::new();
    let mut original = post("1");
    original.favourites_count = 19;
    original.favourited = Some(false);
    let mut boost = post("boost");
    boost.reblog = Some(Box::new(original.clone()));
    cache.insert_status(boost);
    let mut activity = notification("n1", "mention", "alice", "2026-01-01T12:00:00Z", true);
    activity.status = Some(original.clone());
    cache.insert_notification(activity);
    let mut favorites = Timeline::new(
        Client::new("https://example.org".into(), Some("token".into())),
        TimelineKind::Favorites,
    );
    let _ = favorites.update(Message::PageLoaded(0, Ok(page(&[], "", false))));

    for (liked, count) in [(true, 20), (false, 19), (true, 20)] {
        let _ = favorites.update(Message::Refresh);
        let stale = favorites.request;
        // Apply the confirmed mutation response before changing collection membership.
        let mut confirmed = original.clone();
        confirmed.favourited = Some(liked);
        confirmed.favourites_count = count;
        cache.insert_status(confirmed.clone());
        let _ = favorites.update(Message::CollectionChanged(confirmed));
        let _ = favorites.update(Message::PageLoaded(stale, Ok(page(&["1"], "1", false))));

        assert_eq!(favorites.statuses.contains(&"1".to_owned()), liked);
        assert!(
            cosmic::iced::advanced::subscription::into_recipes(favorites.subscription()).is_empty()
        );
        for post in [
            &cache.statuses["1"],
            cache.statuses["boost"].reblog.as_deref().unwrap(),
            cache.notifications["n1"].status.as_ref().unwrap(),
        ] {
            assert_eq!(post.favourited, Some(liked));
            assert_eq!(post.favourites_count, count);
        }
    }
}

#[test]
fn favorite_requests_reconcile_stale_counts_from_successful_http_responses() {
    check_reaction_responses(false, false);
}

#[test]
fn boost_requests_reconcile_stale_counts_from_successful_http_responses() {
    check_reaction_responses(true, false);
}

#[test]
fn boost_requests_reconcile_counts_inside_reblog_wrappers() {
    check_reaction_responses(true, true);
}

fn check_reaction_responses(boost: bool, wrapped: bool) {
    use std::io::{Read, Write};
    // was liked, previous count, returned count, returned state, expected count
    let cases = [
        (false, 19, 20, true, 20),
        (true, 20, 20, false, 19),
        (false, 19, 20, true, 20),
        (true, 20, 19, false, 19),
        (false, 19, 19, true, 20),
        (true, 0, 0, false, 0),
        (false, 20, 20, false, 20),
        (true, 20, 25, false, 25),
    ];
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for (was_liked, _, returned_count, returned_state, _) in cases {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let read = stream.read(&mut buffer).unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
            }
            let action = match (boost, was_liked) {
                (true, true) => "unreblog",
                (true, false) => "reblog",
                (false, true) => "unfavourite",
                (false, false) => "favourite",
            };
            assert!(String::from_utf8_lossy(&request)
                .starts_with(&format!("POST /api/v1/statuses/1/{action} HTTP/1.1")));
            let mut response = post("1");
            if boost {
                response.reblogs_count = returned_count;
                response.reblogged = Some(returned_state);
                if wrapped && !was_liked {
                    let mut wrapper = post("wrapper");
                    wrapper.reblog = Some(Box::new(response));
                    response = wrapper;
                }
            } else {
                response.favourites_count = returned_count;
                response.favourited = Some(returned_state);
            }
            let body = serde_json::to_string(&response).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    let client = Client::new(url, Some("test-token".into()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for (was_liked, previous_count, _, returned_state, expected_count) in cases {
            let status = if boost {
                crate::features::status::boost::set(
                    &client,
                    "1".into(),
                    was_liked,
                    Some(previous_count),
                )
                .await
            } else {
                crate::features::status::favorite::set(
                    &client,
                    "1".into(),
                    was_liked,
                    Some(previous_count),
                )
                .await
            }
            .unwrap();
            let mut cache = Cache::new();
            cache.insert_status(status);
            let original = &cache.statuses["1"];
            if boost {
                assert_eq!(original.reblogged, Some(returned_state));
                assert_eq!(original.reblogs_count, expected_count);
                if wrapped && !was_liked {
                    assert_eq!(
                        cache.statuses["wrapper"]
                            .reblog
                            .as_ref()
                            .unwrap()
                            .reblogs_count,
                        expected_count
                    );
                }
            } else {
                assert_eq!(original.favourited, Some(returned_state));
                assert_eq!(original.favourites_count, expected_count);
            }
        }
    });
    server.join().unwrap();
}

#[test]
fn paging_keeps_the_latest_loaded_window_and_bounds_stream_buffers() {
    let mut feed = feed();
    let initial: Vec<_> = (0..crate::cache::MAX_RETAINED_ITEMS)
        .map(|id| post(&format!("a{id}")))
        .collect();
    let _ = feed.update(Message::PageLoaded(
        0,
        Ok(fetch::Page {
            statuses: initial,
            cursor: Some("older".into()),
            more: true,
        }),
    ));
    let _ = feed.update(Message::LoadMore);
    let _ = feed.update(Message::PageLoaded(
        feed.request,
        Ok(fetch::Page {
            statuses: (0..20).map(|id| post(&format!("b{id}"))).collect(),
            cursor: Some("end".into()),
            more: true,
        }),
    ));
    assert_eq!(feed.statuses.len(), crate::cache::MAX_RETAINED_ITEMS);
    assert_eq!(feed.statuses.front().map(String::as_str), Some("a20"));
    assert_eq!(feed.statuses.back().map(String::as_str), Some("b19"));
    for id in 0..crate::cache::MAX_BUFFERED_ITEMS + 20 {
        let _ = feed.update(Message::IncomingStatus(post(&format!("live{id}"))));
    }
    assert_eq!(feed.incoming.len(), crate::cache::MAX_BUFFERED_ITEMS);
    let _ = feed.update(Message::PublishNew);
    assert_eq!(feed.statuses.len(), crate::cache::MAX_RETAINED_ITEMS);
    assert_eq!(feed.statuses.front().map(String::as_str), Some("live119"));
}

#[test]
fn cache_pruning_preserves_boost_originals_reader_posts_and_deletion_tombstones() {
    let mut cache = Cache::new();
    let mut original = post("original");
    original.account.avatar = "keep-avatar".into();
    let mut boost = post("boost");
    boost.reblog = Some(Box::new(original));
    cache.insert_status(boost);
    cache.insert_status(post("reader"));
    cache.insert_status(post("unused"));
    cache.deleted.insert("deleted".into());
    cache.insert_handle("keep-avatar".into(), crate::cache::fallback_handle());
    cache.insert_handle("unused-avatar".into(), crate::cache::fallback_handle());
    cache.retain_referenced(
        HashSet::from(["boost".into(), "reader".into()]),
        &HashSet::new(),
        HashSet::new(),
        HashSet::new(),
    );
    assert_eq!(cache.statuses.len(), 3);
    assert!(cache.statuses.contains_key("original"));
    assert!(cache.statuses.contains_key("reader"));
    assert!(!cache.statuses.contains_key("unused"));
    assert!(cache.handles.contains_key("keep-avatar"));
    assert!(!cache.handles.contains_key("unused-avatar"));
    cache.insert_status(post("deleted"));
    assert!(!cache.statuses.contains_key("deleted"));
}

#[test]
fn scoped_snapshots_preserve_the_selected_feed_and_its_current_content() {
    let root = std::env::temp_dir().join(format!("momo-scopes-{}", uuid::Uuid::new_v4()));
    for kind in [
        TimelineKind::Tag("rust".into()),
        TimelineKind::List("friends".into()),
    ] {
        let mut feed = Timeline::new(
            Client::new("https://example.org".into(), None),
            kind.clone(),
        );
        let mut cache = Cache::new();
        let mut current = post("saved");
        current.content = "<p>The latest edit</p>".into();
        cache.insert_status(current);
        let _ = feed.update(Message::PageLoaded(0, Ok(page(&["saved"], "end", false))));
        feed.snapshot(&cache).unwrap().save_in(&root).unwrap();
        let restored: Vec<Status> = serde_json::from_slice(
            &std::fs::read(root.join(format!("{}.json", kind.slug()))).unwrap(),
        )
        .unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, "saved");
        assert_eq!(restored[0].content, "<p>The latest edit</p>");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scoped_snapshot_names_are_single_bounded_path_components() {
    for kind in [
        TimelineKind::Tag("../../private".into()),
        TimelineKind::List("../other".into()),
        TimelineKind::Tag("字".repeat(500)),
    ] {
        let slug = kind.slug();
        assert_eq!(std::path::Path::new(&slug).components().count(), 1);
        assert!(slug.len() < 32);
    }
    assert_ne!(
        TimelineKind::Tag("rust".into()).slug(),
        TimelineKind::Tag("Rust".into()).slug()
    );
    assert_ne!(
        TimelineKind::Tag("same".into()).slug(),
        TimelineKind::List("same".into()).slug()
    );
}
