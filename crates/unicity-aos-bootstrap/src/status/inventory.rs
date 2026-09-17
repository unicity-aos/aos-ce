use astrid_core::kernel_api::KernelResponse;
use serde::Serialize;

/// Principal-visible package metadata, never an effective-grant assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapsuleInventory {
    principal: String,
    #[serde(flatten)]
    content: InventoryContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum InventoryContent {
    Available { capsules: Vec<CapsuleSummary> },
    Stopped,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CapsuleSummary {
    name: String,
    version: String,
    description: Option<String>,
}

impl CapsuleInventory {
    pub(super) fn stopped(principal: String) -> Self {
        Self {
            principal,
            content: InventoryContent::Stopped,
        }
    }

    pub(super) fn from_response(principal: String, response: Option<KernelResponse>) -> Self {
        let content = match response {
            Some(KernelResponse::CapsuleMetadata(entries)) => {
                let mut capsules: Vec<_> = entries
                    .into_iter()
                    .map(|entry| CapsuleSummary {
                        name: entry.name,
                        version: entry.version,
                        description: entry.description,
                    })
                    .collect();
                capsules.sort_by(|a, b| a.name.cmp(&b.name));
                InventoryContent::Available { capsules }
            }
            _ => InventoryContent::Unavailable,
        };
        Self { principal, content }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_only_package_summary_and_sorts_names() {
        let entry = |name| {
            serde_json::from_value(serde_json::json!({
                "name": name, "version": "1.2.3", "description": "Package description",
                "interceptor_events": [], "env": {}, "capabilities": {"network": ["example.com"]}
            }))
            .unwrap()
        };
        let value = CapsuleInventory::from_response(
            "alice".into(),
            Some(KernelResponse::CapsuleMetadata(vec![
                entry("z"),
                entry("a"),
            ])),
        );
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["capsules"][0]["name"], "a");
        assert_eq!(json["capsules"][0]["version"], "1.2.3");
        assert_eq!(json["capsules"][0].as_object().unwrap().len(), 3);
        assert!(json["capsules"][0].get("capabilities").is_none());
    }

    #[test]
    fn stopped_and_denied_are_not_empty_inventory() {
        for inventory in [
            CapsuleInventory::stopped("alice".into()),
            CapsuleInventory::from_response("alice".into(), None),
            CapsuleInventory::from_response(
                "alice".into(),
                Some(KernelResponse::Error("denied".into())),
            ),
        ] {
            let json = serde_json::to_value(inventory).unwrap();
            assert_eq!(json["principal"], "alice");
            assert_ne!(json["state"], "available");
            assert!(json.get("capsules").is_none());
        }
    }

    #[test]
    fn successful_empty_inventory_is_explicit() {
        let value = CapsuleInventory::from_response(
            "alice".into(),
            Some(KernelResponse::CapsuleMetadata(vec![])),
        );
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["state"], "available");
        assert_eq!(json["capsules"], serde_json::json!([]));
    }
}
