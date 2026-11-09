//! Durable private drafts and owned copies of selected files. Feed cache
//! cleanup never touches this directory.
use super::{Limits, Media, State};
use crate::fl;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Store {
    root: Option<PathBuf>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            root: dirs::data_local_dir().map(|dir| dir.join("dev.edfloreshz.Toot").join("drafts")),
        }
    }
}
impl Store {
    #[cfg(test)]
    pub fn at(root: PathBuf) -> Self {
        Self { root: Some(root) }
    }
    fn directory(&self, account: &str) -> Result<PathBuf, String> {
        let key: String = account
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if key.is_empty() {
            return Err(fl!("compose-storage-error"));
        }
        let mut directory = self
            .root
            .as_ref()
            .ok_or_else(|| fl!("compose-storage-error"))?
            .clone();
        for chunk in key.as_bytes().chunks(128) {
            // Hex encoding contains only ASCII bytes.
            directory.push(std::str::from_utf8(chunk).map_err(|_| fl!("compose-storage-error"))?);
        }
        Ok(directory)
    }
    fn draft_dir(&self, account: &str, draft: &str) -> Result<PathBuf, String> {
        if uuid::Uuid::parse_str(draft).is_err() {
            return Err(fl!("compose-storage-error"));
        }
        Ok(self.directory(account)?.join(draft))
    }
    pub fn asset_path(&self, account: &str, draft: &str, name: &str) -> Result<PathBuf, String> {
        let path = Path::new(name);
        if path.components().count() != 1
            || !path
                .file_stem()
                .and_then(|value| value.to_str())
                .is_some_and(|value| uuid::Uuid::parse_str(value).is_ok())
            || !matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("png" | "jpg" | "gif" | "webp")
            )
        {
            return Err(fl!("compose-storage-error"));
        }
        Ok(self.draft_dir(account, draft)?.join(name))
    }
    pub fn load(&self, account: &str) -> Result<Vec<State>, String> {
        let path = self.directory(account)?.join("drafts.json");
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(_) => return Err(fl!("compose-storage-error")),
        };
        let mut drafts: Vec<State> =
            serde_json::from_slice(&bytes).map_err(|_| fl!("compose-storage-error"))?;
        for draft in &mut drafts {
            self.draft_dir(account, &draft.id)?;
            for media in &draft.media {
                if let Some(name) = &media.stored_name {
                    self.asset_path(account, &draft.id, name)?;
                }
            }
            if draft.attempted_at.is_some() {
                draft.error = Some(fl!("compose-uncertain"));
            }
        }
        drafts.retain(|draft| draft.published_id.is_none());
        Ok(drafts)
    }
    pub fn save(&self, account: &str, drafts: &[State]) -> Result<(), String> {
        let dir = self.directory(account)?;
        private_dir(&dir).map_err(|_| fl!("compose-storage-error"))?;
        let bytes = serde_json::to_vec(drafts).map_err(|_| fl!("compose-storage-error"))?;
        let temporary = dir.join(format!("{}.new", uuid::Uuid::new_v4()));
        let result = (|| -> std::io::Result<()> {
            let mut file = private_file(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, dir.join("drafts.json"))?;
            #[cfg(unix)]
            fs::File::open(&dir)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result.map_err(|_| fl!("compose-storage-error"))
    }
    pub fn remove_assets(&self, account: &str, draft: &str) {
        if let Ok(path) = self.draft_dir(account, draft) {
            let _ = fs::remove_dir_all(path);
        }
    }
    pub fn import(
        &self,
        account: &str,
        draft: &str,
        paths: &[PathBuf],
        limits: &Limits,
    ) -> Result<Vec<Media>, String> {
        let dir = self.draft_dir(account, draft)?;
        private_dir(&dir).map_err(|_| fl!("compose-storage-error"))?;
        let mut imported = vec![];
        for path in paths {
            let mut file = fs::File::open(path).map_err(|_| fl!("compose-file-error"))?;
            if file
                .metadata()
                .map_err(|_| fl!("compose-file-error"))?
                .len()
                > limits.image_bytes
            {
                return Err(fl!("compose-image-too-large"));
            }
            let mut header = [0; 16];
            let read = file
                .read(&mut header)
                .map_err(|_| fl!("compose-file-error"))?;
            let (mime, extension) =
                image_format(&header[..read]).ok_or_else(|| fl!("compose-image-type"))?;
            if !limits.image_mimes.iter().any(|value| value == mime) {
                return Err(fl!("compose-image-type"));
            }
            file.seek(SeekFrom::Start(0))
                .map_err(|_| fl!("compose-file-error"))?;
            let id = uuid::Uuid::new_v4().to_string();
            let stored_name = format!("{id}.{extension}");
            let mut copy =
                private_file(&dir.join(&stored_name)).map_err(|_| fl!("compose-storage-error"))?;
            let bytes = std::io::copy(
                &mut file.take(limits.image_bytes.saturating_add(1)),
                &mut copy,
            )
            .map_err(|_| fl!("compose-file-error"))?;
            copy.sync_all().map_err(|_| fl!("compose-storage-error"))?;
            if bytes > limits.image_bytes {
                let _ = fs::remove_file(dir.join(&stored_name));
                return Err(fl!("compose-image-too-large"));
            }
            imported.push(Media {
                id,
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| stored_name.clone()),
                stored_name: Some(stored_name),
                bytes,
                description: String::new(),
                attachment: None,
                uploading: false,
                progress: Default::default(),
                error: None,
            });
        }
        #[cfg(unix)]
        fs::File::open(&dir)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| fl!("compose-storage-error"))?;
        Ok(imported)
    }
}
fn private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
pub fn image_format(header: &[u8]) -> Option<(&'static str, &'static str)> {
    if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if header.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("image/jpeg", "jpg"))
    } else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        Some(("image/gif", "gif"))
    } else if header.starts_with(b"RIFF") && header.get(8..12) == Some(b"WEBP") {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}
