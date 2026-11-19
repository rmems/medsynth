//! Record-level metadata labels.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A label in a record's metadata — exceptional conditions the contract
/// requires to be visible on the record itself.
///
/// This is a closed set: new labels are added here first, not written as
/// free-form strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordLabel {
    /// The record is retained even though every one of its anchors cites a
    /// superseded source — the only case where contract §7 lets such a
    /// record stay in the dataset, and only when this label is present.
    HistoricalSuperseded,
}
