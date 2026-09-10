//! Browser entry points. The page/worker owns persistence of the stored
//! records and the Google identity binding; this module only validates records
//! and runs the shared in-memory history contract.
use crate::history::{
    Binding, Command, Error, Reply,
    memory::{MemoryJournal, StoredRecord},
};
use uuid::Uuid;
use wasm_bindgen::prelude::*;

/// Structural metadata validation only; it cannot enroll or apply a profile.
#[wasm_bindgen]
pub fn validate_profile_operation(bytes: &[u8]) -> Result<Vec<u8>, JsError> {
    crate::Operation::decode(bytes)
        .and_then(|v| v.encode())
        .map_err(|e| JsError::new(&e.to_string()))
}

#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Outcome {
    Ok { value: Reply },
    Error { kind: &'static str, message: String },
}
fn outcome(result: Result<Reply, Error>) -> String {
    let outcome = match result {
        Ok(value) => Outcome::Ok { value },
        Err(error) => Outcome::Error {
            kind: error.kind(),
            message: error.to_string(),
        },
    };
    serde_json::to_string(&outcome).unwrap_or_else(|_| {
        "{\"status\":\"error\",\"kind\":\"storage\",\"message\":\"The profile history could not be encoded.\"}".into()
    })
}

/// One in-memory history journal. The caller restores it from its own stored
/// records and persists `record()` after every accepted write.
#[wasm_bindgen]
pub struct ProfileHistory {
    journal: MemoryJournal,
}
#[wasm_bindgen]
impl ProfileHistory {
    #[wasm_bindgen(constructor)]
    pub fn new(binding: &str, device: &str, records: &str) -> Result<ProfileHistory, JsError> {
        let binding: Binding =
            serde_json::from_str(binding).map_err(|_| JsError::new(&Error::Binding.to_string()))?;
        let device =
            Uuid::parse_str(device).map_err(|_| JsError::new(&Error::Binding.to_string()))?;
        let records: Vec<StoredRecord> =
            serde_json::from_str(records).map_err(|_| JsError::new(&Error::Storage.to_string()))?;
        let journal = MemoryJournal::restore(binding, device, records)
            .map_err(|e| JsError::new(&format!("{}:{e}", e.kind())))?;
        Ok(Self { journal })
    }
    /// Execute one JSON `Command`; returns `{status:"ok",value:Reply}` or
    /// `{status:"error",kind,message}`. A failed command changes nothing.
    pub fn execute(&mut self, command: &str) -> String {
        let command: Command = match serde_json::from_str(command) {
            Ok(command) => command,
            Err(_) => return outcome(Err(crate::Error::Invalid.into())),
        };
        outcome(self.journal.execute(command))
    }
    pub fn overview(&self) -> Result<String, JsError> {
        let overview = self
            .journal
            .overview()
            .map_err(|e| JsError::new(&e.to_string()))?;
        serde_json::to_string(&overview).map_err(|_| JsError::new(&Error::Storage.to_string()))
    }
    /// The durable form of one operation, for the caller's store.
    pub fn record(&self, operation: &str) -> Result<String, JsError> {
        let operation =
            Uuid::parse_str(operation).map_err(|_| JsError::new(&Error::Changed.to_string()))?;
        let record = self
            .journal
            .record(operation)
            .map_err(|e| JsError::new(&e.to_string()))?;
        serde_json::to_string(&record).map_err(|_| JsError::new(&Error::Storage.to_string()))
    }
    pub fn device(&self) -> String {
        self.journal.device().to_string()
    }
}
