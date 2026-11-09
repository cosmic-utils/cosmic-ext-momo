use super::*;
use crate::client::Client;
use serde_json::json;

fn account(id: &str, privacy: &str) -> Account {
    serde_json::from_value(json!({"id":id,"username":id,"acct":id,"display_name":id,"locked":false,
        "created_at":"2020-01-01T00:00:00Z","followers_count":0,"following_count":0,"statuses_count":0,
        "note":"","url":format!("https://example.org/@{id}"),"avatar":"","avatar_static":"","header":"","header_static":"","emojis":[],"fields":[],"bot":false,
        "source":{"privacy":privacy,"sensitive":true,"language":"es","note":""}})).unwrap()
}
fn post(visibility: StatusVisibility) -> Status {
    serde_json::from_value(json!({"id":"s1","uri":"https://example.org/posts/s1","account":account("alice","public"),"content":"<p>Rendered text</p>",
        "created_at":"2020-01-01T00:00:00Z","emojis":[],"replies_count":0,"reblogs_count":0,"favourites_count":0,"sensitive":false,"spoiler_text":"Warning",
        "visibility":visibility,"media_attachments":[],"mentions":[],"tags":[],"quote_approval":{"automatic":[],"manual":[],"current_user":"unknown"}})).unwrap()
}
struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("toot-compose-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn character_count_uses_graphemes_url_reservations_and_remote_mention_concessions() {
    assert_eq!(count::count("e\u{301}👩‍👩‍👧‍👦", "", 23), 2);
    assert_eq!(
        count::count("https://example.org/a-very-long-url", "", 23),
        23
    );
    assert_eq!(count::count("(https://example.org/path).", "", 23), 26);
    assert_eq!(count::count("https://example.org/a_(b)", "", 10), 10);
    assert_eq!(count::count("@alice@example.org hi", "", 23), 9);
    assert_eq!(count::count("mail alice@example.org", "", 23), 22);
    assert_eq!(count::count("hi", "https://example.org", 23), 21);
    assert_eq!(count::count("example.org", "", 23), 11);
    assert_eq!(count::count("\u{301}", "a", 23), 1);
}

#[test]
fn replies_never_default_to_a_broader_audience_and_honor_account_preferences() {
    for original in [
        StatusVisibility::Public,
        StatusVisibility::Unlisted,
        StatusVisibility::Private,
        StatusVisibility::Direct,
    ] {
        for preference in ["public", "unlisted", "private", "direct"] {
            let me = account("me", preference);
            let target = post(original.clone());
            let reply = State::reply_to(&target, Some(&me));
            assert!(visibility_rank(&reply.visibility) >= visibility_rank(&original));
            assert!(
                visibility_rank(&reply.visibility)
                    >= visibility_rank(&State::new(Some(&me)).visibility)
            );
            assert_eq!(reply.spoiler_text, "Warning");
            assert!(reply.content_warning);
            assert_eq!(reply.text, "@alice@example.org ");
        }
    }
    let mut me = account("me", "public");
    me.source = None;
    assert_eq!(State::new(Some(&me)).visibility, StatusVisibility::Private);
}

#[test]
fn replies_deduplicate_recipients_and_exclude_the_authenticated_account() {
    let mut target = post(StatusVisibility::Direct);
    target.mentions=serde_json::from_value(json!([
        {"id":"alice","username":"alice","acct":"alice","url":"https://example.org/@alice"},
        {"id":"me","username":"me","acct":"me","url":"https://example.org/@me"},
        {"id":"bob","username":"bob","acct":"bob@remote.example","url":"https://remote.example/@bob"}
    ])).unwrap();
    let reply = State::reply_to(&target, Some(&account("me", "public")));
    assert_eq!(reply.text, "@alice@example.org @bob@remote.example ");
    target.account = account("me", "public");
    target.mentions.clear();
    assert_eq!(
        State::reply_to(&target, Some(&account("me", "public"))).text,
        ""
    );
}

#[test]
fn editing_uses_original_source_and_leaves_existing_polls_and_visibility_unchanged() {
    let original = post(StatusVisibility::Private);
    let source = megalodon::entities::StatusSource {
        id: "s1".into(),
        text: "Original https://example.org".into(),
        spoiler_text: String::new(),
    };
    let mut state = State::edit(&original, source);
    assert_eq!(state.text, "Original https://example.org");
    assert_eq!(state.edit_id.as_deref(), Some("s1"));
    let payload = state.payload();
    assert!(payload.get("visibility").is_none());
    assert!(payload.get("poll").is_none());
    assert!(payload.get("media_ids").is_none());
    assert_eq!(payload["spoiler_text"], "");
    state.had_media = true;
    assert_eq!(state.payload()["media_ids"], json!([]));
}

#[test]
fn uncertain_submissions_keep_the_original_key_and_freeze_edits_until_resolved() {
    let mut state = State::default();
    state.text = "hello".into();
    state.changed();
    let key = state.submission_key.clone();
    state.attempted_at = Some(chrono::Utc::now().timestamp());
    state.sending = true;
    state.failed(network::SubmitError::Uncertain);
    assert!(!state.editable());
    assert!(state.retry_safe());
    assert_eq!(state.submission_key, key);
    let restored: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert!(!restored.editable());
    assert_eq!(restored.submission_key, key);
    assert_eq!(restored.text, state.text);
    state.attempted_at = Some(chrono::Utc::now().timestamp() - 60 * 60);
    assert!(!state.retry_safe());
    state.failed(network::SubmitError::Rejected("rejected".into()));
    assert!(state.editable());
    assert_eq!(state.text, "hello");
}

#[test]
fn status_requests_send_idempotency_keys_only_for_new_posts_and_preserve_reply_privacy() {
    let client = Client::new("https://example.org".into(), Some("test-token".into()));
    let http = reqwest::Client::new();
    let mut state = State::reply_to(
        &post(StatusVisibility::Private),
        Some(&account("me", "public")),
    );
    let request = network::status_request(&http, &client, &state).unwrap();
    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(
        request.url().as_str(),
        "https://example.org/api/v1/statuses"
    );
    assert_eq!(request.headers()["Idempotency-Key"], state.submission_key);
    let payload: serde_json::Value =
        serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
    assert_eq!(payload["visibility"], "private");
    assert_eq!(payload["in_reply_to_id"], "s1");
    state.edit_id = Some("s1".into());
    let request = network::status_request(&http, &client, &state).unwrap();
    assert_eq!(request.method(), reqwest::Method::PUT);
    assert!(request.headers().get("Idempotency-Key").is_none());
}

#[test]
fn drafts_and_image_copies_survive_restarts_and_are_isolated_per_account() {
    let directory = Temporary::new();
    let store = store::Store::at(directory.0.join("drafts"));
    let path = directory.0.join("original.png");
    std::fs::write(&path, include_bytes!("../../../assets/missing.png")).unwrap();
    let mut draft = State::default();
    draft.text = "Private draft".into();
    draft.dirty = true;
    draft.media = store
        .import(
            "https://example.org#1",
            &draft.id,
            &[path.clone()],
            &Limits::default(),
        )
        .unwrap();
    draft.media[0].description = "Description".into();
    store
        .save("https://example.org#1", &[draft.clone()])
        .unwrap();
    let restored = store.load("https://example.org#1").unwrap();
    assert_eq!(restored[0].text, draft.text);
    assert_eq!(restored[0].media[0].description, "Description");
    assert!(store.load("https://example.org#2").unwrap().is_empty());
    assert!(store
        .asset_path(
            "https://example.org#1",
            &draft.id,
            draft.media[0].stored_name.as_ref().unwrap()
        )
        .unwrap()
        .exists());
    store.remove_assets("https://example.org#1", &draft.id);
    assert!(path.exists());
    assert!(store
        .asset_path("account", "../../escape", "file.png")
        .is_err());
    assert!(store
        .asset_path("account", &draft.id, "../../escape.png")
        .is_err());
}

#[test]
fn draft_storage_reports_corruption_and_never_restores_confirmed_posts_for_resubmission() {
    let directory = Temporary::new();
    let store = store::Store::at(directory.0.clone());
    let mut draft = State::default();
    draft.text = "Already sent".into();
    draft.published_id = Some("s1".into());
    store.save("account", &[draft.clone()]).unwrap();
    assert!(store.load("account").unwrap().is_empty());
    assert!(!draft.valid(&Limits::default()));
    let snapshot = std::fs::read_dir(&directory.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("drafts.json");
    std::fs::write(&snapshot, b"broken json").unwrap();
    assert!(store.load("account").is_err());
    assert_eq!(std::fs::read(snapshot).unwrap(), b"broken json");
    let bad_root = directory.0.join("not-a-directory");
    std::fs::write(&bad_root, b"existing").unwrap();
    assert!(store::Store::at(bad_root.clone())
        .save("account", &[State::default()])
        .is_err());
    assert_eq!(std::fs::read(bad_root).unwrap(), b"existing");
}

#[test]
fn imports_validate_image_content_and_enforce_server_limits() {
    let directory = Temporary::new();
    let store = store::Store::at(directory.0.join("drafts"));
    let state = State::default();
    let fake = directory.0.join("fake.png");
    std::fs::write(&fake, b"not an image").unwrap();
    assert!(store
        .import("account", &state.id, &[fake], &Limits::default())
        .is_err());
    let image = directory.0.join("photo.png");
    std::fs::write(&image, include_bytes!("../../../assets/missing.png")).unwrap();
    let mut limits = Limits::default();
    limits.image_bytes = 1;
    assert!(store
        .import("account", &state.id, &[image.clone()], &limits)
        .is_err());
    limits = Limits::default();
    limits.image_mimes = vec!["image/jpeg".into()];
    assert!(store
        .import("account", &state.id, &[image], &limits)
        .is_err());
}

#[test]
fn composer_validation_supports_media_only_posts_and_rejects_broader_replies() {
    let mut state = State::reply_to(
        &post(StatusVisibility::Private),
        Some(&account("me", "public")),
    );
    state.visibility = StatusVisibility::Public;
    assert!(!state.valid(&Limits::default()));
    let mut state = State::default();
    assert!(!state.valid(&Limits::default()));
    let attachment: Attachment =
        serde_json::from_value(json!({"id":"m1","type":"image","url":"https://example.org/photo"}))
            .unwrap();
    state.media.push(Media {
        id: uuid::Uuid::new_v4().to_string(),
        name: "image.png".into(),
        stored_name: None,
        bytes: 0,
        description: "Alt".into(),
        attachment: Some(attachment),
        uploading: false,
        progress: Default::default(),
        error: None,
    });
    assert!(state.valid(&Limits::default()));
    state.media[0].uploading = true;
    assert!(!state.valid(&Limits::default()));
    state.media[0].uploading = false;
    state.media[0].description = "x".repeat(1501);
    assert!(!state.valid(&Limits::default()));
}

#[test]
fn posting_limits_use_instance_configuration() {
    let limits = Limits::from_json(
        &json!({"configuration":{"statuses":{"max_characters":1000,"characters_reserved_per_url":30,"max_media_attachments":8},"media_attachments":{"image_size_limit":2000,"description_limit":50,"supported_mime_types":["image/png"]}}}),
    );
    assert_eq!(
        (limits.characters, limits.reserved_url, limits.attachments),
        (1000, 30, 8)
    );
    assert_eq!(limits.image_bytes, 2000);
    assert_eq!(limits.description, 50);
    assert_eq!(limits.image_mimes, vec!["image/png"]);
}

#[test]
fn durable_storage_handles_long_account_keys_without_exposing_files() {
    let directory = Temporary::new();
    let store = store::Store::at(directory.0.join("drafts"));
    let key = format!("https://{}.example.org#account", "a".repeat(220));
    let mut state = State::default();
    state.text = "Private text".into();
    let original = directory.0.join("image.png");
    std::fs::write(&original, include_bytes!("../../../assets/missing.png")).unwrap();
    state.media = store
        .import(&key, &state.id, &[original], &Limits::default())
        .unwrap();
    store.save(&key, &[state.clone()]).unwrap();
    assert_eq!(store.load(&key).unwrap()[0].text, state.text);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = store
            .asset_path(
                &key,
                &state.id,
                state.media[0].stored_name.as_ref().unwrap(),
            )
            .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}

#[test]
fn editing_existing_attachments_uses_cached_remote_previews_without_local_files() {
    let mut post = post(StatusVisibility::Public);
    post.media_attachments = serde_json::from_value(json!([
        {"id":"image", "type":"image", "url":"https://example.org/original.png", "preview_url":"https://example.org/thumb.png"},
        {"id":"no-preview", "type":"image", "url":"https://example.org/fallback.png"},
        {"id":"video", "type":"video", "url":"https://example.org/video.mp4", "preview_url":"https://example.org/video.jpg"},
        {"id":"no-video-preview", "type":"video", "url":"https://example.org/no-preview.mp4"}
    ])).unwrap();
    let state = State::edit(
        &post,
        megalodon::entities::StatusSource {
            id: post.id.clone(),
            text: "Text".into(),
            spoiler_text: String::new(),
        },
    );
    assert_eq!(
        state.preview_urls(),
        vec![
            "https://example.org/thumb.png",
            "https://example.org/fallback.png",
            "https://example.org/video.jpg"
        ]
    );
    let client = Client::new("https://example.org".into(), None);
    let temporary = Temporary::new();
    let store = store::Store::at(temporary.0.clone());
    let mut cache = crate::cache::Cache::new();
    for (index, url) in state.preview_urls().iter().enumerate() {
        assert!(state.media[index].stored_name.is_none());
        assert!(preview_handle(&state, &state.media[index], &client, &store, &cache).is_none());
        cache
            .handles
            .insert(url.clone(), crate::cache::fallback_handle());
        assert!(preview_handle(&state, &state.media[index], &client, &store, &cache).is_some());
    }
}

#[test]
fn new_polls_enforce_server_limits_and_keep_existing_poll_edits_untouched() {
    let limits = Limits::default();
    let mut state = State::default();
    state.text = "Choose".into();
    state.poll_options = vec!["One".into(), "Two".into()];
    assert!(state.valid(&limits));
    assert_eq!(state.payload()["poll"]["options"], json!(["One", "Two"]));
    state.poll_options[1] = " one ".into();
    assert!(!state.valid(&limits));
    state.poll_options[1] = "".into();
    assert!(!state.valid(&limits));
    state.poll_options[1] = "Two".into();
    state.poll_duration = 1;
    assert!(!state.valid(&limits));
    state.poll_duration = 86400;
    state.poll_options.push("x".repeat(51));
    assert!(!state.valid(&limits));
    state.poll_options.pop();
    let mut edited = State::edit(
        &post(StatusVisibility::Public),
        megalodon::entities::StatusSource {
            id: "s1".into(),
            text: "edited".into(),
            spoiler_text: "".into(),
        },
    );
    edited.has_poll = true;
    assert!(edited.payload().get("poll").is_none());
    edited.poll_options = state.poll_options;
    assert!(!edited.valid(&limits));
}

#[test]
fn schedule_requests_cannot_silently_become_immediate_posts_and_keep_idempotency() {
    let client = crate::client::Client::new("https://example.org".into(), Some("secret".into()));
    let http = reqwest::Client::new();
    let mut state = State::default();
    state.text = "Later".into();
    state.schedule = "yesterday".into();
    assert!(!state.valid(&Limits::default()));
    assert!(network::status_request(&http, &client, &state).is_err());
    state.schedule = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    assert!(state.valid(&Limits::default()));
    let request = network::status_request(&http, &client, &state).unwrap();
    assert_eq!(request.headers()["idempotency-key"], state.submission_key);
    let payload: serde_json::Value =
        serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
    assert!(payload["scheduled_at"].as_str().is_some());
    let accepted = network::decode_published(
        json!({"id":"schedule-1","scheduled_at":state.schedule,"params":{"text":"Later"}}),
    )
    .unwrap();
    assert!(accepted.scheduled && accepted.post.is_none());
    assert_eq!(accepted.id, "schedule-1");
}

#[test]
fn private_messages_require_recipients_and_drafts_keep_poll_and_schedule_state() {
    let mut state = State::default();
    state.visibility = StatusVisibility::Direct;
    state.reply_ceiling = Some(StatusVisibility::Direct);
    state.text = "No recipient".into();
    assert!(!state.valid(&Limits::default()));
    state.text = "@alice@example.org Hello".into();
    assert!(state.valid(&Limits::default()));
    assert_eq!(state.payload()["visibility"], "direct");
    state.visibility = StatusVisibility::Public;
    assert!(!state.valid(&Limits::default()));
    state.visibility = StatusVisibility::Direct;
    state.poll_options = vec!["Yes".into(), "No".into()];
    state.poll_multiple = true;
    state.schedule = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let directory = Temporary::new();
    let store = store::Store::at(directory.0.join("drafts"));
    store
        .save("https://example.org#alice", &[state.clone()])
        .unwrap();
    let restored = store.load("https://example.org#alice").unwrap().remove(0);
    assert_eq!(restored.poll_options, state.poll_options);
    assert!(restored.poll_multiple);
    assert_eq!(restored.schedule, state.schedule);
    assert_eq!(restored.reply_ceiling, Some(StatusVisibility::Direct));
    assert!(store.load("https://example.org#bob").unwrap().is_empty());
}
