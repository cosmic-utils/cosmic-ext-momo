# Release implementation plan

Each milestone receives its own local commit. Changes must not be pushed without explicit authorization.

- [x] 1. Account identity and isolation: instance + account ID, isolated caches/preferences, reset views, reject stale responses.
- [x] 2. Welcome wizard: server validation, browser authorization, verified account confirmation, optional additional accounts, guest browsing, Settings integration.
- [x] 3. Reading: content warnings/sensitive media, conversations, profile posts/pins, links/addresses/timestamps/selection, descriptions/polls, reporting/filters.
- [x] 4. Posting: durable drafts, recoverable submission, discard confirmation, reply addressing/privacy, account identity, character counting, uploads/alt text, editing.
- [x] 5. Beta readiness implementation: refresh/retry, streaming recovery/edits/deletions, credential errors, cache controls, accessibility/localization review, CI, packaging/documentation.
- [x] 6. Stable-release feature implementation: unread/desktop notifications, search resolution/pagination, list management, profile editing/follower browsers, autocomplete/polls/scheduling/private conversations.

## Verification

Compile and format every milestone. Add focused tests for privacy, account isolation, data loss, and pagination. Live server, keychain, graphical accessibility, and clean-install checks require an actual desktop/test account; document verification limits instead of claiming they ran.

## Phase 5 implementation and verification

- Added cancellable TLS streaming with bounded reconnect backoff, a recovery notice, catch-up requests, streamed edits/deletions, and token-safe authorization headers.
- Added deletion tombstones and synchronized edits across cached boosts/notifications; late responses cannot restore deleted content or overwrite newer edits.
- Added account credential recovery, bounded discovery/search/image requests, retry controls, stale-search protection, and cache cleanup that keeps drafts/accounts/preferences.
- Reviewed accessible labels and localized settings, profile/discovery controls, and recovery copy. Removed duplicate Fluent IDs and added fallback/coverage checks. Existing non-English translations remain partial.
- Added CI, validated desktop/AppStream metadata, install/staging support, and build/testing/translation documentation.
- Local validation: formatting, locked/offline all-target compilation, 61 passing tests, development build, resource checks, and staged install/uninstall verification.
- Live server reconnection/token revocation, keyring integration, graphical accessibility, and clean-distribution installation remain manual release checks in `docs/BETA_TESTING.md`. Phase 5 implementation is complete; this is not a declaration that those release checks passed.

## Phase 6 implementation and verification

- Added device/account-scoped unread tracking and desktop alerts over the freedesktop notification service. Alerts are enabled by default for the active account while Toot runs, can be disabled in Settings, and omit all post/private-message content. Cached history does not generate a notification storm.
- Added per-category search pagination, remote resolution for authenticated searches, deduplication, same-page retries, and stale-query protection.
- Added list creation/renaming/deletion confirmation, paginated members, and a following-only member chooser. Added multiline bio/display-name/follow-approval editing and paginated follower/following browsers.
- Added debounced, cursor-aware mention/hashtag suggestions; new single/multiple-choice polls with server limits; server-side scheduling and scheduled-post rescheduling/cancellation. These compose fields survive draft persistence. Invalid schedules cannot silently publish immediately; existing poll edits stay unchanged.
- Added private conversation browsing/read state and recipient-only compose entry from profiles. Private message drafts keep a direct audience and require a recipient.
- Local validation: 74 tests, including a temporary private D-Bus delivery test; locked/offline all-target compilation, formatting, development build, and resource validation. The private-bus test requires local socket access; it does not display a real desktop notification.
- Live desktop presentation/Do Not Disturb, real server mutations, remote resolution, scheduled delivery after quitting, keyring integration, and graphical accessibility remain release acceptance checks in `docs/BETA_TESTING.md`. Implementation completion does not certify those checks or a stable release.
