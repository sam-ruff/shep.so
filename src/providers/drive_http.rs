//! Shared bounded HTTP decoding for Drive backups and profile operations.
use reqwest::Response;
pub(crate) const JSON_LIMIT: u64 = 2 * 1024 * 1024;

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 512
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

pub(crate) async fn response_bytes(mut response: Response, limit: u64) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        response.status().is_success(),
        "Google Drive request failed (HTTP {}).",
        response.status()
    );
    anyhow::ensure!(
        response.content_length().is_none_or(|size| size <= limit),
        "Google Drive response exceeds the size limit."
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() as u64 + chunk.len() as u64 <= limit,
            "Google Drive response exceeds the size limit."
        );
        bytes.extend(chunk);
    }
    Ok(bytes)
}

pub(crate) async fn response_json(response: Response) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::from_slice(
        &response_bytes(response, JSON_LIMIT).await?,
    )?)
}
