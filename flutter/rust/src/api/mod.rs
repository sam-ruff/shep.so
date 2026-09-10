use crate::{database::Database, operations};
use flutter_rust_bridge::frb;
use std::sync::Arc;

/// One profile per running application. Credentials enter individual requests
/// from the platform keychain and are never saved by Rust.
#[frb(opaque)]
pub struct MobileProfile {
    pub(crate) database: Arc<Database>,
    pub(crate) operations: Arc<operations::Operations>,
}
impl MobileProfile {
    pub async fn open(path: String) -> anyhow::Result<Self> {
        let database = Database::open(path).await?;
        Ok(Self {
            operations: database.operations.clone(),
            database,
        })
    }
    pub async fn request(&self, json: String) -> anyhow::Result<String> {
        // Parsing and all business logic runs in Rust's background executor.
        // Only controlled user-facing error messages cross the bridge.
        let request = serde_json::from_str(&json)
            .map_err(|_| anyhow::anyhow!("Invalid mail request. Update Shep and retry."))?;
        let profile = Self {
            database: self.database.clone(),
            operations: self.operations.clone(),
        };
        // Cancellation of a Dart waiter must not detach protocol work from its
        // account lock or capacity permits. The owned operation runs to completion.
        let data = tokio::spawn(async move { operations::run(&profile, request).await })
            .await.unwrap_or_else(|_| Err(anyhow::anyhow!("The operation stopped unexpectedly. Refresh mail or check delivery status before retrying.")));
        Ok(serde_json::to_string(&match data {
            Ok(value) => serde_json::json!({"data":value}),
            Err(error) => serde_json::json!({"error":error.to_string()}),
        })?)
    }
}
#[frb(init)]
pub fn initialize() {
    // Deliberately do not install a protocol logger or global panic dump.
}
