# Beta verification

Phase 5 adds recovery and release infrastructure. Passing automated checks does not establish that every live server, keyring, codec, or assistive technology works.

## Automated coverage

The Rust tests cover account isolation, stale responses, retry and pagination state, streamed deletion tombstones, edits in boosts and notifications, cache cleanup boundaries, credential error classification, streaming event normalization and reconnect backoff, draft recovery, posting privacy, filtering, and media playback using generated local media.

CI checks formatting, compilation, tests, translation fallback keys, desktop/AppStream metadata, and installation into a temporary staging directory.

## Desktop checks before distributing a beta

These require a real desktop and test account; they have not been performed as part of the headless implementation checks.

- Sign in, restart, switch accounts on the same server, and cancel setup. Check both a locked and an unlocked keyring; keyring failures should remain recoverable.
- Disconnect the network while reading and loading older posts. Confirm existing posts remain, retry works, and reconnecting live updates catch up without mixing account data.
- From another client, edit and delete a post, including a boosted post and one shown in a notification or open thread. Confirm edited warnings and sensitive media need revealing again, and deleted content does not return from a late response.
- Revoke a test account's token. Confirm the sign-in notice appears, reconnect attempts stop, and signing in again preserves drafts. Temporary network errors and permission errors must not remove accounts.
- Clear the cache in Settings. Confirm the active account's snapshots and shared downloaded images disappear, while other accounts' feed snapshots, saved accounts, preferences, and draft attachments remain. Open posts stay visible; refresh downloads their images again.
- Use the app with keyboard navigation and a screen reader. Check onboarding, menus, composer, retry controls, image descriptions, media controls, and the context drawer. Check light/dark themes and a narrow window. Icon controls have accessible names; this still needs graphical verification.
- Try a translated locale and confirm missing messages fall back to English. Existing translations are partial; do not describe them as complete.
- Install on a clean target distribution. Check the app launcher icon, AppStream listing, keyring integration, and video codecs, then uninstall. A staged install alone does not verify these.

## Known boundaries

Recovery reloads the newest home and notification pages; it does not promise a complete history of every event missed during a long outage. Older home posts remain reachable through pagination. Public feeds, profiles, and search can be refreshed or reopened for server-side changes missed offline.

Phase 6 adds desktop/unread notifications, paginated search with remote resolution, list/member editing, display-name/bio/follow-approval editing, follower/following browsers, completion, new polls, server-side scheduling, and private conversations. These additions still need live acceptance testing before a stable release.

Unread counts are local to this device and account, with a bounded history. Visiting Notifications marks the inbox read locally. Desktop alerts apply only to the active account while Toot is running; they do not run after quitting or bypass the desktop’s Do Not Disturb settings. Alerts contain actor/event summaries, not post or private-message content. Scheduling is performed by the Mastodon server and continues when Toot is closed.

## Phase 6 acceptance checks (not yet performed on a live desktop/server)

- Receive mentions, follows, and boosts with Toot minimized. Check the desktop alert and unread badge, disable alerts in Settings, and try Do Not Disturb. Switch accounts and restart: counts must remain isolated, and replayed events must not produce duplicate alerts. Check missed activity after reconnecting.
- Paste a remote handle and post URL into the appropriate search tab. Load multiple pages, change tabs/queries during requests, and retry an offline page. Results must not mix or repeat indefinitely.
- Create/rename a list, add someone you follow, remove a member, page through members, and cancel/confirm deletion. A failed action must leave the editor recoverable; check the server before repeating a timed-out creation.
- Edit a multiline bio, display name, and follow-approval setting. Check that unrelated profile metadata and images remain intact. Browse follower/following pages and return from an opened profile.
- Complete mentions and hashtags in the middle of Unicode text. Move the cursor, switch drafts/accounts, and check that stale suggestions cannot overwrite text.
- Create single/multiple-choice polls at your server’s limits. Try blank/duplicate options, overlong options, and attachments. Restart with a poll draft and confirm all options remain.
- Schedule a post at least five minutes ahead using local time. Restart with a scheduled draft, verify server acceptance without adding a fake published post to Home, then reschedule/cancel from the app menu. Test publication after quitting Toot and recovery after an uncertain submission.
- Start a private message from a profile, verify its recipient-only audience, reply from Private conversations, and test unread state. Check content warnings/filters, and confirm desktop alerts do not disclose private text.

Automated notification delivery is tested against a temporary private D-Bus service. That verifies the wire payload without displaying a real alert; actual COSMIC notification presentation remains a desktop acceptance check.
