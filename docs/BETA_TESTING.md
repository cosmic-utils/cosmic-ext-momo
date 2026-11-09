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

Desktop/unread notifications, search pagination and resolution, list editing, profile editing, autocomplete, new polls, scheduling, and private conversations belong to phase 6.
