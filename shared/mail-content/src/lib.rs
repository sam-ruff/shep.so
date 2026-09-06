//! Cached mail decoding for native Rust and browser workers. No credentials,
//! network, filesystem, platform UI or server storage dependency.
pub mod attachments;
pub const MAX_MESSAGE_BYTES: usize = 25 * 1024 * 1024;

#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    pub fn attachment_catalog(raw: &[u8]) -> Result<String, JsError> {
        let files = super::attachments::catalog(raw).map_err(|e| JsError::new(&e.to_string()))?;
        serde_json::to_string(&files).map_err(|e| JsError::new(&e.to_string()))
    }
    #[wasm_bindgen]
    pub fn attachment_bytes(raw: &[u8], id: &str) -> Result<Vec<u8>, JsError> {
        super::attachments::read(raw, id)
            .map(|(_, bytes)| bytes)
            .map_err(|e| JsError::new(&e.to_string()))
    }
    #[wasm_bindgen]
    pub fn attachment_filename(value: &str) -> String {
        super::attachments::filename(value)
    }
}
