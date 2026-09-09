//! Read-only release-test probe for the daemon-owned distro lock.

use astrid_core::PrincipalId;
use astrid_core::kernel_api::{AdminRequestKind, AdminResponseBody};
use astrid_uplink::admin_client::AdminClient;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let principal = PrincipalId::default();
    let mut client = AdminClient::connect(principal.clone()).await?;
    match client
        .request(AdminRequestKind::DistroLockGet { principal })
        .await?
    {
        AdminResponseBody::DistroLock(lock) => {
            let lock = lock.ok_or("default principal has no durable distro provenance")?;
            println!("{}", serde_json::to_string(&lock)?);
            Ok(())
        }
        response => Err(format!("unexpected distro provenance response: {response:?}").into()),
    }
}
