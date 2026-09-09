use super::*;

pub(super) fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
pub(super) fn page_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 2048 && !value.chars().any(char::is_control)
}
pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn uuid(value: &Value) -> Result<Uuid> {
    let text = value.as_str().ok_or(Error::Invalid)?;
    let id = Uuid::parse_str(text).map_err(|_| Error::Invalid)?;
    if id.is_nil() || id.to_string() != text {
        return Err(Error::Invalid);
    }
    Ok(id)
}
pub(super) fn file(value: &Value, principal: &str, namespace: &str) -> Result<File> {
    let props = &value["appProperties"];
    if value["trashed"] != false
        || value["ownedByMe"] != true
        || value["spaces"] != json!(["appDataFolder"])
        || value["mimeType"] != "application/json"
        || props["shepType"] != "profile"
    {
        return Err(Error::Invalid);
    }
    if props["shepFormat"] != "operation-v1" {
        return Err(crate::Error::Upgrade.into());
    }
    if props["shepNamespace"] != sha256(namespace.as_bytes()) {
        return Err(Error::Namespace);
    }
    let id = value["id"]
        .as_str()
        .filter(|id| self::id(id))
        .ok_or(Error::Invalid)?;
    let operation = uuid(&props["shepOperation"])?;
    if value["name"] != format!("shep-profile-{operation}.json") {
        return Err(Error::Invalid);
    }
    let size = value["size"]
        .as_str()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|size| *size > 0 && *size <= crate::MAX_RECORD_BYTES)
        .ok_or(Error::TooLarge)?;
    let sha256 = props["shepSha256"]
        .as_str()
        .filter(|s| digest(s))
        .ok_or(Error::Invalid)?;
    if value.get("sha256Checksum").is_some_and(|v| v != sha256) {
        return Err(Error::Changed);
    }
    Ok(File {
        principal: principal.into(),
        namespace: namespace.into(),
        id: id.into(),
        profile: uuid(&props["shepProfile"])?,
        generation: uuid(&props["shepGeneration"])?,
        operation,
        size,
        sha256: sha256.into(),
    })
}

pub(super) fn status_error(status: StatusCode) -> Error {
    match status.as_u16() {
        401 => Error::Authorization,
        403 => Error::Denied,
        _ => Error::Http(status.as_u16()),
    }
}
pub(super) async fn bytes(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    if response.status() != StatusCode::OK {
        return Err(status_error(response.status()));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(Error::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
        if chunk.len() > limit - bytes.len() {
            return Err(Error::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub(super) async fn json(response: Response) -> Result<Value> {
    // The common parser rejects duplicate keys and excessive nesting. Provider
    // JSON is bounded independently of profile operation bytes and mail data.
    crate::json::decode(&bytes(response, JSON_LIMIT).await?).map_err(|_| Error::Invalid)
}
fn upload_metadata(file: &File) -> Value {
    json!({
        "id": file.id,
        "name": format!("shep-profile-{}.json", file.operation),
        "mimeType": "application/json",
        "parents": ["appDataFolder"],
        "appProperties": {
            "shepType": "profile",
            "shepFormat": "operation-v1",
            "shepNamespace": sha256(file.namespace.as_bytes()),
            "shepProfile": file.profile,
            "shepGeneration": file.generation,
            "shepOperation": file.operation,
            "shepSha256": file.sha256,
        },
    })
}
/// Rebuild only validated fields for the device-local catalog. No token, provider
/// error body or unbounded optional Drive metadata is copied into this record.
pub(super) fn saved_file(file: &File) -> String {
    let mut metadata = upload_metadata(file);
    metadata["spaces"] = json!(["appDataFolder"]);
    metadata["ownedByMe"] = json!(true);
    metadata["trashed"] = json!(false);
    metadata["size"] = json!(file.size.to_string());
    metadata.to_string()
}
pub(super) fn multipart(file: &File, record: &[u8]) -> Result<(String, Vec<u8>)> {
    if !id(&file.id) || record.len() > crate::MAX_RECORD_BYTES {
        return Err(Error::Invalid);
    }
    let metadata = serde_json::to_vec(&upload_metadata(file)).map_err(|_| Error::Invalid)?;
    let boundary = format!("shep_{}", Uuid::new_v4().simple());
    if record
        .windows(boundary.len())
        .any(|w| w == boundary.as_bytes())
    {
        // A new attempt may choose another boundary; immutable media stays exact.
        return Err(Error::Invalid);
    }
    let mut body = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n")
        .into_bytes();
    body.extend(metadata);
    body.extend(format!("\r\n--{boundary}\r\nContent-Type: application/json\r\n\r\n").bytes());
    body.extend(record);
    body.extend(format!("\r\n--{boundary}--\r\n").bytes());
    Ok((format!("multipart/related; boundary={boundary}"), body))
}
