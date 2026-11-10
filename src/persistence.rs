//! Disk-persisted snapshots of feed content, so a restart can render the
//! last-seen posts/notifications immediately instead of a blank screen while
//! the network catches up.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use cosmic::Application;
use serde::{de::DeserializeOwned, Serialize};

use crate::app::AppModel;

/// Cap on how many items are persisted per feed, matching existing
/// pagination page sizes.
pub const MAX_STATUSES: usize = 200;
pub const MAX_NOTIFICATIONS: usize = 100;
/// Cap on how many distinct hashtag/list snapshot files are kept.
const MAX_SCOPED_FILES: usize = 5;

pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IMAGE_CACHE_BYTES: u64 = 256 * 1024 * 1024;

fn cache_root() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join(AppModel::APP_ID))
}

/// Hex-encode an account key into a filesystem-safe directory name. Every byte
/// is encoded because punctuation replacement can collide across identities.
fn account_slug(base_url: &str) -> String {
    base_url
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn account_dir(base_url: &str) -> Option<PathBuf> {
    Some(cache_root()?.join(account_slug(base_url)))
}

/// Path for a named feed snapshot within an account's cache directory.
fn snapshot_path(base_url: &str, slug: &str) -> Option<PathBuf> {
    Some(account_dir(base_url)?.join(format!("{slug}.json")))
}

/// Save a bounded slice of items to disk as JSON. Errors are logged, not
/// surfaced — a failed cache write should never interrupt the user.
fn save_snapshot<T: Serialize>(
    base_url: &str,
    slug: &str,
    items: &[T],
    max_items: usize,
) -> std::io::Result<()> {
    let path = snapshot_path(base_url, slug)
        .ok_or_else(|| std::io::Error::other("cache directory unavailable"))?;
    let bounded = &items[..max_items.min(items.len())];
    let data = serde_json::to_vec(bounded).map_err(std::io::Error::other)?;
    write_snapshot(&path, &data)
}

pub enum Snapshot {
    Statuses(String, Vec<megalodon::entities::Status>),
    Notifications(Vec<megalodon::entities::Notification>),
}

static SNAPSHOT_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SNAPSHOT_WRITTEN: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, u64>>,
> = std::sync::LazyLock::new(Default::default);

pub fn reserve_snapshot_save() -> u64 {
    SNAPSHOT_REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

pub fn save_snapshots(
    account: &str,
    revision: u64,
    snapshots: Vec<Snapshot>,
) -> Result<(), String> {
    let mut written = SNAPSHOT_WRITTEN
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if written.get(account).is_some_and(|saved| *saved > revision) {
        return Ok(());
    }
    written.insert(account.to_owned(), revision);
    for snapshot in snapshots {
        let result = match snapshot {
            Snapshot::Statuses(slug, posts) => save_snapshot(account, &slug, &posts, MAX_STATUSES),
            Snapshot::Notifications(items) => {
                save_snapshot(account, "notifications", &items, MAX_NOTIFICATIONS)
            }
        };
        result.map_err(|error| error.to_string())?;
    }
    prune_scoped_snapshots(account);
    Ok(())
}

fn private_directory(path: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    restrict_permissions(path, 0o700)
}

fn restrict_permissions(path: &std::path::Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn write_snapshot(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("missing cache directory"))?;
    private_directory(parent)?;
    let temporary = parent.join(format!("{}.new", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&temporary)?.write_all(data)?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

/// Load a previously-saved snapshot, if any. Returns an empty `Vec` on any
/// error (missing file, corrupt JSON, etc.) — a cold cache is not a failure.
pub fn load_snapshot<T: DeserializeOwned>(base_url: &str, slug: &str) -> Vec<T> {
    let Some(path) = snapshot_path(base_url, slug) else {
        return Vec::new();
    };
    load_snapshot_at(&path)
}

fn load_snapshot_at<T: DeserializeOwned>(path: &std::path::Path) -> Vec<T> {
    let result = (|| {
        restrict_permissions(
            path.parent()
                .ok_or_else(|| std::io::Error::other("missing cache directory"))?,
            0o700,
        )?;
        restrict_permissions(path, 0o600)?;
        std::fs::read(path)
    })();
    result
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

/// Keep only the `MAX_SCOPED_FILES` most-recently-modified hashtag/list
/// snapshots, so browsing many tags/lists doesn't accumulate cache files
/// forever. Called after every status snapshot save (cheap: a handful of
/// directory entries at most).
fn prune_scoped_snapshots(base_url: &str) {
    for prefix in ["tag-", "list-"] {
        prune_prefixed(base_url, prefix);
    }
}

fn prune_prefixed(base_url: &str, prefix: &str) {
    let Some(dir) = account_dir(base_url) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };

    let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((path, modified))
        })
        .collect();

    if files.len() <= MAX_SCOPED_FILES {
        return;
    }

    files.sort_by_key(|(_, modified)| *modified);
    let excess = files.len() - MAX_SCOPED_FILES;
    for (path, _) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

/// Content-addressed disk cache for downloaded images, shared across
/// accounts since media URLs are already public.
fn image_cache_path(url: &str) -> Option<PathBuf> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let hashed = hasher.finish();
    Some(cache_root()?.join("images").join(format!("{hashed:x}")))
}

pub fn load_image(url: &str) -> Option<Vec<u8>> {
    let path = image_cache_path(url)?;
    if std::fs::metadata(&path).ok()?.len() > MAX_IMAGE_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}

pub fn save_image(url: &str, bytes: &[u8], epoch: u64) {
    if let Some(path) = image_cache_path(url) {
        save_image_at(path, bytes, epoch);
    }
}

fn save_image_at(path: PathBuf, bytes: &[u8], epoch: u64) {
    let _guard = IMAGE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    if epoch != image_epoch() {
        return;
    }
    if bytes.len() as u64 <= MAX_IMAGE_BYTES {
        let parent = path.parent().map(std::path::Path::to_path_buf);
        write_image(path, bytes);
        if let Some(parent) = parent {
            prune_images_at(&parent, MAX_IMAGE_CACHE_BYTES);
        }
    }
}

fn prune_images_at(directory: &std::path::Path, budget: u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| (entry.path(), metadata.len(), metadata.modified().ok()))
        })
        .collect();
    let mut bytes: u64 = files.iter().map(|(_, size, _)| size).sum();
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if bytes <= budget {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            bytes = bytes.saturating_sub(size);
        }
    }
}

fn write_image(path: PathBuf, bytes: &[u8]) {
    let Some(parent) = path.parent() else { return };
    if let Err(err) = std::fs::create_dir_all(parent) {
        tracing::warn!("failed to create image cache directory: {err}");
        return;
    }
    if let Err(err) = std::fs::write(&path, bytes) {
        tracing::warn!("failed to write cached image {}: {err}", path.display());
    }
}

/// Remove all cache files for an account, called on logout/account removal.
pub fn clear_account(base_url: &str) {
    let mut written = SNAPSHOT_WRITTEN
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    written.insert(base_url.to_owned(), reserve_snapshot_save());
    if let Some(dir) = account_dir(base_url) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

// Serialize image writes and cleanup. Downloads started before cleanup must not
// repopulate files after the user has cleared them.
static IMAGE_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static IMAGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn image_epoch() -> u64 {
    IMAGE_EPOCH.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn clear_cached_content(account: &str) -> std::io::Result<()> {
    let _guard = IMAGE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    IMAGE_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut written = SNAPSHOT_WRITTEN
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    written.insert(account.to_owned(), reserve_snapshot_save());
    if let Some(root) = cache_root() {
        clear_at(&root, account)?;
    }
    Ok(())
}

fn clear_at(root: &std::path::Path, account: &str) -> std::io::Result<()> {
    remove_cache_dir(root.join(account_slug(account)))?;
    remove_cache_dir(root.join("images"))
}

fn remove_cache_dir(path: PathBuf) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn snapshots_repair_private_permissions_and_replace_content_atomically() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("toot-snapshot-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("home.json");
        std::fs::write(&path, b"[1]").unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(load_snapshot_at::<u32>(&path), vec![1]);
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        write_snapshot(&path, b"[2]").unwrap();
        assert_eq!(load_snapshot_at::<u32>(&path), vec![2]);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn image_cache_removes_old_downloads_to_stay_within_its_byte_budget() {
        let root = std::env::temp_dir().join(format!("toot-images-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        for (index, name) in ["old", "recent", "new"].iter().enumerate() {
            let path = root.join(name);
            std::fs::write(&path, b"data").unwrap();
            std::fs::File::open(path)
                .unwrap()
                .set_modified(
                    std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(index as u64),
                )
                .unwrap();
        }
        prune_images_at(&root, 8);
        assert!(!root.join("old").exists());
        assert!(root.join("recent").exists());
        assert!(root.join("new").exists());
        prune_images_at(&root, 0);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_image_download_from_before_cleanup_cannot_write_back_to_disk() {
        let root = std::env::temp_dir().join(format!("toot-image-test-{}", uuid::Uuid::new_v4()));
        let path = root.join("image");
        save_image_at(path.clone(), b"old", image_epoch().wrapping_sub(1));
        assert!(!path.exists());
        save_image_at(path.clone(), b"current", image_epoch());
        assert_eq!(std::fs::read(&path).unwrap(), b"current");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cache_cleanup_is_account_scoped_and_preserves_drafts_and_other_accounts() {
        let root = std::env::temp_dir().join(format!("toot-cache-test-{}", uuid::Uuid::new_v4()));
        let cache = root.join("cache");
        let alice = "https://example.org#alice";
        let bob = "https://example.org#bob";
        for dir in [
            cache.join(account_slug(alice)),
            cache.join(account_slug(bob)),
            cache.join("images"),
            root.join("drafts"),
        ] {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("saved"), b"keep").unwrap();
        }
        clear_at(&cache, alice).unwrap();
        assert!(!cache.join(account_slug(alice)).exists());
        assert!(!cache.join("images").exists());
        assert!(cache.join(account_slug(bob)).join("saved").exists());
        assert!(root.join("drafts/saved").exists());
        clear_at(&cache, alice).unwrap(); // repeated cleanup is harmless
        std::fs::remove_dir_all(root).unwrap();
    }
}
