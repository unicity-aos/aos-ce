//! Native AOS status over the runtime's typed local control operation.

use std::time::Duration;

use astrid_core::PrincipalId;
use astrid_core::kernel_api::{DaemonStatus, KernelRequest, KernelResponse};
use astrid_uplink::KernelClient;
use serde::Serialize;

const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// Product status derived from the typed runtime status response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AosStatus {
    pub state: &'static str,
    pub pid: u32,
    pub uptime_secs: u64,
    pub runtime_version: String,
    pub ephemeral: bool,
    pub connected_clients: u32,
    pub loaded_capsules: Vec<String>,
}

impl From<DaemonStatus> for AosStatus {
    fn from(status: DaemonStatus) -> Self {
        Self {
            state: "running",
            pid: status.pid,
            uptime_secs: status.uptime_secs,
            runtime_version: status.version,
            ephemeral: status.ephemeral,
            connected_clients: status.connected_clients,
            loaded_capsules: status.loaded_capsules,
        }
    }
}

/// Read status through the typed authenticated local control client.
pub async fn read() -> Result<AosStatus, String> {
    let mut client = tokio::time::timeout(
        STATUS_TIMEOUT,
        KernelClient::connect(PrincipalId::default()),
    )
    .await
    .map_err(|_| "connection timed out".to_owned())?
    .map_err(|_| "could not connect to the local runtime".to_owned())?;

    let response = tokio::time::timeout(STATUS_TIMEOUT, client.request(KernelRequest::GetStatus))
        .await
        .map_err(|_| "status request timed out".to_owned())?
        .map_err(|_| "status request failed".to_owned())?;

    match response {
        KernelResponse::Status(status) => Ok(status.into()),
        KernelResponse::Error(error) => Err(error),
        _ => Err("runtime returned an unexpected status response".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use astrid_core::kernel_api::DaemonStatus;

    use super::AosStatus;

    #[test]
    fn maps_typed_runtime_status_to_product_status() {
        let status = AosStatus::from(DaemonStatus {
            pid: 42,
            uptime_secs: 90,
            version: "0.9.4".to_owned(),
            ephemeral: false,
            connected_clients: 3,
            connections_by_principal: Vec::new(),
            loaded_capsules: vec!["agents".to_owned(), "session".to_owned()],
        });

        assert_eq!(status.state, "running");
        assert_eq!(status.pid, 42);
        assert_eq!(status.runtime_version, "0.9.4");
        assert_eq!(status.loaded_capsules, ["agents", "session"]);
    }

    #[test]
    fn json_has_aos_owned_field_names() {
        let status = AosStatus {
            state: "running",
            pid: 7,
            uptime_secs: 8,
            runtime_version: "0.9.4".to_owned(),
            ephemeral: false,
            connected_clients: 1,
            loaded_capsules: vec!["agents".to_owned()],
        };

        let value = serde_json::to_value(status).expect("serialize status");
        assert_eq!(value["state"], "running");
        assert_eq!(value["runtime_version"], "0.9.4");
        assert!(value.get("astrid").is_none());
    }
}
