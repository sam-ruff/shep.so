use serde::{Deserialize, Serialize};
use serde_json::Value;
use shep_profile_core::SettingKey;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PreferenceState {
    pub values: BTreeMap<SettingKey, Value>,
    pub revisions: BTreeMap<SettingKey, u64>,
}
