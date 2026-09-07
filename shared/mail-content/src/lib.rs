//! Cached mail decoding for native Rust and browser workers. No credentials,
//! network, filesystem, platform UI or server storage dependency.
pub mod attachments;
pub mod document;
pub mod find;
pub mod forwarding;
pub mod mime;
mod plain;
pub mod reader;
pub const MAX_MESSAGE_BYTES: usize = 25 * 1024 * 1024;

#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::prelude::*;

    /// Keep binary files in WASM until the worker copies each into its draft
    /// transaction. Metadata never expands file bytes into JSON number arrays.
    #[wasm_bindgen]
    pub struct PreparedForward(super::forwarding::PreparedForward);

    #[wasm_bindgen]
    impl PreparedForward {
        pub fn metadata(&self) -> Result<String, JsError> {
            serde_json::to_string(&self.0)
                .map_err(|_| JsError::new("Could not return this forward's content."))
        }

        pub fn file_bytes(&self, index: u32) -> Result<Vec<u8>, JsError> {
            self.0
                .files
                .get(index as usize)
                .map(|file| file.bytes.clone())
                .ok_or_else(|| JsError::new("This forward has no attachment at that position."))
        }
    }

    /// Outgoing HTML is not a display document. Callers must still use the
    /// confined message renderer for any preview and free this owned result.
    #[wasm_bindgen]
    pub fn prepare_forward(raw: &[u8]) -> Result<PreparedForward, JsError> {
        super::forwarding::prepare(raw)
            .map(PreparedForward)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen]
    pub fn prepare_message(raw: &[u8], options: &str) -> Result<String, JsError> {
        let options =
            serde_json::from_str(options).map_err(|_| JsError::new("Invalid reader options"))?;
        let prepared =
            super::document::prepare(raw, &options).map_err(|e| JsError::new(&e.to_string()))?;
        serde_json::to_string(&prepared)
            .map_err(|_| JsError::new("Could not return formatted mail"))
    }
    /// A decoding API for workers, not HTML safe to insert into a page. Rendering
    /// must use the separate confined-document preparation layer.
    #[wasm_bindgen]
    pub fn message_body(raw: &[u8]) -> Result<String, JsError> {
        let body = super::reader::decode(raw).map_err(|e| JsError::new(&e.to_string()))?;
        serde_json::to_string(&body)
            .map_err(|_| JsError::new("Could not return this message's body"))
    }
    #[wasm_bindgen]
    pub fn find_text(blocks: &str, query: &str, match_case: bool) -> Result<String, JsError> {
        let blocks: Vec<String> =
            serde_json::from_str(blocks).map_err(|_| JsError::new("Invalid search text"))?;
        let hits = super::find::find(&blocks, query, match_case)
            .map_err(|_| JsError::new("Could not search this message"))?;
        serde_json::to_string(&hits).map_err(|_| JsError::new("Could not return search results"))
    }
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
