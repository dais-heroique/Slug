//! Stable widget identity.

use std::hash::{Hash, Hasher};

/// A stable widget identity.
///
/// `num` is the AccessKit `NodeId` (a hash of the stable key); `key` is the
/// human/agent-stable string the Slug `ref` is derived from. Equal keys always
/// yield equal ids — refs survive re-renders as long as the key is stable.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WidgetId {
    pub num: u64,
    pub key: String,
}

impl WidgetId {
    /// Build an id from a stable key string.
    pub fn from_key(key: impl Into<String>) -> WidgetId {
        let key = key.into();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        WidgetId { num: h.finish().max(1), key }
    }

    /// The AccessKit node id.
    pub fn node_id(&self) -> accesskit::NodeId {
        accesskit::NodeId(self.num)
    }
}
