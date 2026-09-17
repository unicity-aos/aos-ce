#![deny(unsafe_code)]
use astrid_sdk::prelude::*;

#[derive(Default)]
struct Capsule;

#[capsule]
impl Capsule {
    #[astrid::run]
    fn run(&self) -> Result<(), SysError> {
        let requests = ipc::subscribe("cli.v1.command.run.native-input-probe")?;
        runtime::signal_ready()?;
        loop {
            for message in requests.recv(1_000)?.messages {
                let request: serde_json::Value = serde_json::from_str(&message.payload)
                    .map_err(|_| SysError::ApiError("invalid request".into()))?;
                let Some(id) = request.get("req_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                if id.is_empty()
                    || id.len() > 64
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    continue;
                }
                let command = request.get("command").and_then(|v| v.as_str());
                let result = match command {
                    Some("collect") => elicit::secret("token", "Enter a synthetic test token")
                        .and_then(|()| elicit::has_secret("token")),
                    Some("stored") => elicit::has_secret("token"),
                    Some("text") => elicit::text("text", "Enter synthetic text")
                        .map(|value| value == "native-text"),
                    Some("empty-text") => elicit::text("empty-text", "Empty text is valid")
                        .map(|value| value.is_empty()),
                    Some("select") => elicit::select("select", "Choose an option", &["one", "two"])
                        .map(|value| value == "two"),
                    Some("array") => elicit::array("array", "Enter a synthetic list")
                        .map(|value| value == ["one,two", "three"]),
                    Some("empty-array") => elicit::array("empty-array", "Empty list is valid")
                        .map(|value| value.is_empty()),
                    _ => Err(SysError::ApiError("unknown command".into())),
                };
                // Never export the value, even from a synthetic test capsule.
                let ordinary = !matches!(command, Some("collect" | "stored"));
                let (code, output) = match result {
                    Ok(true) if ordinary => (0, "input-matched"),
                    Ok(false) if ordinary => (2, "input-mismatch"),
                    Ok(true) => (0, "secret-present"),
                    Ok(false) => (2, "secret-absent"),
                    Err(_) => (1, "input-not-completed"),
                };
                ipc::publish_json(
                    &format!("cli.v1.command.result.{id}"),
                    &serde_json::json!({"exit_code": code, "output": output, "error": ""}),
                )?;
            }
        }
    }
}
