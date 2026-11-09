use super::{Limits, Media, State};
use crate::{client::Client, fl};
use megalodon::entities::{Attachment, Status};
use std::{
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};

#[derive(Debug, Clone)]
pub enum SubmitError {
    Rejected(String),
    Uncertain,
}
#[derive(Debug, Clone)]
pub struct Published {
    pub id: String,
    pub post: Option<Status>,
}

pub async fn limits(client: Client) -> Result<Limits, String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| fl!("compose-limits-error"))?;
    for endpoint in ["api/v2/instance", "api/v1/instance"] {
        let response = http
            .get(format!(
                "{}/{}",
                client.base_url.trim_end_matches('/'),
                endpoint
            ))
            .send()
            .await
            .map_err(|_| fl!("compose-limits-error"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            continue;
        }
        let bytes = response
            .error_for_status()
            .map_err(|_| fl!("compose-limits-error"))?
            .bytes()
            .await
            .map_err(|_| fl!("compose-limits-error"))?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| fl!("compose-limits-error"))?;
        return Ok(Limits::from_json(&value));
    }
    Err(fl!("compose-limits-error"))
}

pub async fn edit(client: Client, id: String) -> Result<State, String> {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let post = client
            .get_status(id.clone())
            .await
            .map_err(|_| fl!("compose-edit-error"))?
            .json;
        if post.account.id != client.account_id {
            return Err(fl!("compose-edit-error"));
        }
        let source = client
            .get_status_source(id)
            .await
            .map_err(|_| fl!("compose-edit-error"))?
            .json;
        Ok(State::edit(&post, source))
    })
    .await
    .unwrap_or_else(|_| Err(fl!("compose-edit-error")))
}

pub async fn prepare_media(client: Client, state: State) -> Result<Vec<Attachment>, String> {
    let mut prepared = vec![];
    for media in state.media {
        if let Some(mut attachment) = media.attachment {
            if state.edit_id.is_none()
                && attachment.description.as_deref().unwrap_or("") != media.description
            {
                let options = megalodon::megalodon::UpdateMediaInputOptions {
                    description: Some(media.description),
                    ..Default::default()
                };
                attachment = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    client.update_media(attachment.id.clone(), Some(&options)),
                )
                .await
                .ok()
                .and_then(Result::ok)
                .ok_or_else(|| fl!("compose-alt-error"))?
                .json;
            }
            prepared.push(attachment);
        }
    }
    Ok(prepared)
}

pub async fn submit(client: Client, state: State) -> Result<Published, SubmitError> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|_| SubmitError::Rejected(fl!("compose-post-error")))?;
    let request = status_request(&http, &client, &state)?;
    let response = http
        .execute(request)
        .await
        .map_err(|_| SubmitError::Uncertain)?;
    if !response.status().is_success() {
        if response.status().is_client_error() && !matches!(response.status().as_u16(), 408 | 409) {
            let details = response
                .bytes()
                .await
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|value| {
                    value["error"]
                        .as_str()
                        .map(|text| text.chars().take(500).collect::<String>())
                });
            return Err(SubmitError::Rejected(match details {
                Some(details) => format!("{} {}", fl!("compose-post-error"), details),
                None => fl!("compose-post-error"),
            }));
        }
        return Err(SubmitError::Uncertain);
    }
    let bytes = response.bytes().await.map_err(|_| SubmitError::Uncertain)?;
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| SubmitError::Uncertain)?;
    let id = value["id"]
        .as_str()
        .ok_or(SubmitError::Uncertain)?
        .to_string();
    if value
        .get("quote_approval")
        .is_none_or(serde_json::Value::is_null)
    {
        value["quote_approval"] =
            serde_json::to_value(megalodon::entities::QuoteApproval::default())
                .map_err(|_| SubmitError::Uncertain)?;
    }
    let post = serde_json::from_value(value).ok();
    Ok(Published { id, post })
}

pub fn status_request(
    http: &reqwest::Client,
    client: &Client,
    state: &State,
) -> Result<reqwest::Request, SubmitError> {
    let mut url = reqwest::Url::parse(&client.base_url)
        .map_err(|_| SubmitError::Rejected(fl!("compose-post-error")))?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| SubmitError::Rejected(fl!("compose-post-error")))?;
        path.clear().extend(["api", "v1", "statuses"]);
        if let Some(id) = &state.edit_id {
            path.push(id);
        }
    }
    let request = if state.edit_id.is_some() {
        http.put(url)
    } else {
        http.post(url)
            .header("Idempotency-Key", state.submission_key.clone())
    };
    let body = serde_json::to_vec(&state.payload())
        .map_err(|_| SubmitError::Rejected(fl!("compose-post-error")))?;
    request
        .bearer_auth(client.token.as_deref().unwrap_or_default())
        .header("Content-Type", "application/json")
        .body(body)
        .build()
        .map_err(|_| SubmitError::Rejected(fl!("compose-post-error")))
}

struct ProgressReader<R> {
    inner: R,
    progress: Arc<AtomicU64>,
}
impl<R: AsyncRead + Unpin> AsyncRead for ProgressReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buffer);
        if let Poll::Ready(Ok(())) = &result {
            self.progress
                .fetch_add((buffer.filled().len() - before) as u64, Ordering::Relaxed);
        }
        result
    }
}
pub async fn upload(client: Client, path: PathBuf, media: Media) -> Result<Attachment, String> {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|_| fl!("compose-file-error"))?;
        let reader = ProgressReader {
            inner: file,
            progress: media.progress,
        };
        let options = megalodon::megalodon::UploadMediaInputOptions {
            description: Some(media.description),
            ..Default::default()
        };
        let response = client
            .upload_media_reader(
                Box::new(reader),
                Some(&options),
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
            )
            .await
            .map_err(|_| fl!("compose-upload-error"))?;
        let id = match response.json {
            megalodon::entities::UploadMedia::Attachment(attachment) => return Ok(attachment),
            megalodon::entities::UploadMedia::AsyncAttachment(attachment) => attachment.id,
        };
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if let Ok(response) = client.get_media(id.clone()).await {
                if !response.json.url.is_empty() {
                    return Ok(response.json);
                }
            }
        }
        Err(fl!("compose-upload-error"))
    })
    .await
    .unwrap_or_else(|_| Err(fl!("compose-upload-error")))
}

#[cfg(target_os = "linux")]
pub async fn pick_images() -> Result<Vec<PathBuf>, String> {
    use cosmic::dialog::file_chooser;
    let filter = file_chooser::FileFilter::new(&fl!("compose-images")).mimetype("image/*");
    match file_chooser::open::Dialog::new()
        .title(fl!("compose-add-images"))
        .filter(filter)
        .open_files()
        .await
    {
        Ok(response) => response
            .urls()
            .iter()
            .map(|url| url.to_file_path().map_err(|_| fl!("compose-file-error")))
            .collect(),
        Err(file_chooser::Error::Cancelled) => Ok(vec![]),
        Err(_) => Err(fl!("compose-picker-error")),
    }
}
#[cfg(not(target_os = "linux"))]
pub async fn pick_images() -> Result<Vec<PathBuf>, String> {
    Err(fl!("compose-picker-error"))
}
