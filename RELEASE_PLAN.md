# Release implementation plan

Each milestone receives its own local commit. Changes must not be pushed without explicit authorization.

- [x] 1. Account identity and isolation: instance + account ID, isolated caches/preferences, reset views, reject stale responses.
- [x] 2. Welcome wizard: server validation, browser authorization, verified account confirmation, optional additional accounts, guest browsing, Settings integration.
- [x] 3. Reading: content warnings/sensitive media, conversations, profile posts/pins, links/addresses/timestamps/selection, descriptions/polls, reporting/filters.
- [x] 4. Posting: durable drafts, recoverable submission, discard confirmation, reply addressing/privacy, account identity, character counting, uploads/alt text, editing.
- [x] 5. Beta readiness implementation: refresh/retry, streaming recovery/edits/deletions, credential errors, cache controls, accessibility/localization review, CI, packaging/documentation.
- [ ] 6. Stable-release features: unread/desktop notifications, search resolution/pagination, list management, profile editing/follower browsers, autocomplete/polls/scheduling/private conversations.

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
