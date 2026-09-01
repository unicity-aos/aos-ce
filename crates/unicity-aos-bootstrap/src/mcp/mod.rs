//! Product-owned MCP endpoint and local interaction bridge.
//!
//! The pinned runtime's MCP shim remains a compatibility transport for this
//! release. AOS owns the externally visible command, server identity, and
//! interaction policy. That lets hosts without MCP form elicitation use a
//! trusted local decision surface without weakening or forking the runtime.

mod interaction;

use std::ffi::OsString;
use std::process::{ExitCode, ExitStatus, Stdio};

use clap::{Args, ValueEnum};
use serde_json::{Map, Value};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc::UnboundedReceiver;

use unicity_aos_bootstrap::AosHome;

#[cfg(unix)]
mod unix_signals {
    use std::os::fd::AsRawFd as _;
    use std::os::raw::c_int;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicI32, Ordering};

    use tokio::sync::mpsc;

    const SIGHUP: c_int = 1;
    const SIGINT: c_int = 2;
    const SIGTERM: c_int = 15;

    static SIGNAL_SINK: AtomicI32 = AtomicI32::new(-1);

    type SignalHandler = unsafe extern "C" fn(c_int);

    unsafe extern "C" {
        fn signal(number: c_int, handler: SignalHandler) -> usize;
        fn write(file_descriptor: c_int, buffer: *const u8, count: usize) -> isize;
    }

    unsafe extern "C" fn note_signal(number: c_int) {
        let sink = SIGNAL_SINK.load(Ordering::Acquire);
        if sink >= 0 {
            let byte = [number as u8];
            unsafe {
                let _ = write(sink, byte.as_ptr(), 1);
            }
        }
    }

    pub(super) fn interrupt_receiver() -> mpsc::UnboundedReceiver<super::InterruptSignal> {
        let (stream_read, stream_write) = UnixStream::pair().expect("create signal pipe");
        let (sender, receiver) = mpsc::unbounded_channel();
        let sink = stream_write.as_raw_fd();
        SIGNAL_SINK.store(sink, Ordering::Release);
        std::mem::forget(stream_write);

        let mut stream_read = stream_read;
        std::thread::spawn(move || {
            let mut signal = [0_u8; 1];
            while let Ok(1) = std::io::Read::read(&mut stream_read, &mut signal) {
                let Some(signal) = super::InterruptSignal::from_raw(i32::from(signal[0])) else {
                    continue;
                };
                if sender.send(signal).is_err() {
                    break;
                }
            }
        });

        unsafe {
            signal(SIGHUP, note_signal);
            signal(SIGINT, note_signal);
            signal(SIGTERM, note_signal);
        }
        receiver
    }
}

#[cfg(unix)]
fn interrupt_receiver() -> UnboundedReceiver<InterruptSignal> {
    unix_signals::interrupt_receiver()
}

#[cfg(not(unix))]
fn interrupt_receiver() -> UnboundedReceiver<InterruptSignal> {
    let (_, receiver) = tokio::sync::mpsc::unbounded_channel();
    receiver
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterruptSignal {
    Hangup,
    Interrupt,
    Terminate,
}

impl InterruptSignal {
    fn from_raw(number: i32) -> Option<Self> {
        match number {
            1 => Some(Self::Hangup),
            2 => Some(Self::Interrupt),
            15 => Some(Self::Terminate),
            _ => None,
        }
    }

    fn exit_code(self) -> ExitCode {
        #[cfg(unix)]
        let number = match self {
            Self::Hangup => 1,
            Self::Interrupt => 2,
            Self::Terminate => 15,
        };

        #[cfg(unix)]
        return ExitCode::from(128 + number);

        #[cfg(not(unix))]
        {
            let _ = self;
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug)]
enum ServeFailure {
    Exit(ExitStatus),
    Interrupted(InterruptSignal),
    Io(String),
}

#[derive(Debug, Args)]
pub(crate) struct ServeArgs {
    /// Choose where constrained approval forms are presented.
    #[arg(long, value_enum, default_value_t = InteractionMode::Auto)]
    interaction: InteractionMode,
    /// Project directory the host attached to this runtime session.
    #[arg(long, value_name = "PATH")]
    workspace: Option<std::path::PathBuf>,
    /// Runtime request timeout forwarded exactly by the product bridge.
    #[arg(long = "request-timeout", value_name = "DURATION")]
    request_timeout: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum InteractionMode {
    /// Prefer the MCP client, falling back to the trusted local AOS provider.
    #[default]
    Auto,
    /// Require the MCP client to present interactions.
    Client,
    /// Always present constrained decisions through the local AOS provider.
    Native,
    /// Refuse interactive requests.
    Deny,
}

pub(crate) fn handle_serve(principal: Option<String>, args: ServeArgs) -> ExitCode {
    let home = match AosHome::resolve() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("aos mcp serve: failed to resolve product home: {error}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("aos mcp serve: failed to start async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve(&home, principal.as_deref(), &args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(ServeFailure::Exit(status)) => {
            eprintln!("aos mcp serve: bundled MCP transport exited with {status}");
            transport_exit_code(status)
        }
        Err(ServeFailure::Interrupted(signal)) => signal.exit_code(),
        Err(ServeFailure::Io(error)) => {
            eprintln!("aos mcp serve: {error}");
            ExitCode::FAILURE
        }
    }
}

fn runtime_arguments(principal: Option<&str>, args: &ServeArgs) -> Vec<OsString> {
    let mut arguments = Vec::<OsString>::new();
    if let Some(principal) = principal {
        arguments.push(OsString::from("--principal"));
        arguments.push(OsString::from(principal));
    }
    arguments.extend([OsString::from("mcp"), OsString::from("serve")]);
    if let Some(workspace) = args.workspace.as_ref() {
        arguments.push(OsString::from("--workspace"));
        arguments.push(workspace.as_os_str().to_os_string());
    }
    if let Some(timeout) = args.request_timeout.as_ref() {
        arguments.push(OsString::from("--request-timeout"));
        arguments.push(OsString::from(timeout));
    }
    arguments
}

async fn serve(
    home: &AosHome,
    principal: Option<&str>,
    args: &ServeArgs,
) -> Result<(), ServeFailure> {
    let mode = args.interaction;
    let runtime_args = runtime_arguments(principal, args);

    let mut standard_command = home
        .runtime_command_with_args(&runtime_args)
        .map_err(|error| ServeFailure::Io(format!("failed to prepare bundled runtime: {error}")))?;
    standard_command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut command = tokio::process::Command::from(standard_command);
    command.kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        ServeFailure::Io(format!(
            "failed to start bundled MCP compatibility transport: {error}"
        ))
    })?;
    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| ServeFailure::Io("bundled MCP transport did not expose stdin".to_owned()))?;
    let child_stdout = child.stdout.take().ok_or_else(|| {
        ServeFailure::Io("bundled MCP transport did not expose stdout".to_owned())
    })?;

    let mut upstream = BufReader::new(tokio::io::stdin());
    let mut downstream = BufReader::new(child_stdout);
    let mut client_frame = Vec::new();
    let mut transport_frame = Vec::new();
    let mut upstream_out = tokio::io::stdout();
    let mut downstream_in = Some(child_stdin);
    let mut client_supports_form = false;
    let mut presenter = interaction::NativePresenter;
    let mut upstream_open = true;
    let mut interrupt_rx = Some(interrupt_receiver());

    loop {
        tokio::select! {
            result = read_frame(&mut upstream, &mut client_frame), if upstream_open => {
                let bytes_read = result.map_err(|error| {
                    ServeFailure::Io(format!("failed to read MCP client: {error}"))
                })?;
                if bytes_read == 0 {
                    downstream_in.take();
                    upstream_open = false;
                    continue;
                }
                let Some(forwarded) =
                    prepare_client_message(&client_frame, mode, &mut client_supports_form)
                else {
                    let transport_input = downstream_in.as_mut().ok_or_else(|| {
                        ServeFailure::Io("bundled MCP transport input is closed".to_owned())
                    })?;
                    write_frame(transport_input, &client_frame)
                        .await
                        .map_err(|error| {
                            ServeFailure::Io(format!(
                                "failed to write bundled MCP transport: {error}"
                            ))
                        })?;
                    client_frame.clear();
                    continue;
                };
                let transport_input = downstream_in.as_mut().ok_or_else(|| {
                    ServeFailure::Io("bundled MCP transport input is closed".to_owned())
                })?;
                write_frame(transport_input, &forwarded).await.map_err(|error| {
                    ServeFailure::Io(format!("failed to write bundled MCP transport: {error}"))
                })?;
                client_frame.clear();
            },
            result = read_frame(&mut downstream, &mut transport_frame) => {
                let bytes_read = result.map_err(|error| {
                    ServeFailure::Io(format!(
                        "failed to read bundled MCP transport: {error}"
                    ))
                })?;
                if bytes_read == 0 {
                    break;
                }
                match transport_action(&transport_frame, mode, client_supports_form) {
                    TransportAction::Forward => {
                        write_frame(&mut upstream_out, &transport_frame)
                            .await
                            .map_err(|error| {
                                ServeFailure::Io(format!(
                                    "failed to write MCP client: {error}"
                                ))
                            })?;
                    },
                    TransportAction::Rewrite(mut message) => {
                        rewrite_server_identity(&mut message);
                        let frame = json_frame(&message).ok_or_else(|| {
                            ServeFailure::Io("failed to encode initialize response".to_owned())
                        })?;
                        write_frame(&mut upstream_out, &frame).await.map_err(|error| {
                            ServeFailure::Io(format!("failed to write MCP client: {error}"))
                        })?;
                    },
                    TransportAction::Present(request) => {
                        let response = match interaction::resolve(&request, &mut presenter) {
                            Ok(response) => response,
                            Err(error) => {
                                eprintln!("aos mcp serve: local interaction denied: {error}");
                                interaction::cancelled_response(&request).ok_or_else(|| {
                                    ServeFailure::Io(
                                        "elicitation request did not carry a response id".to_owned(),
                                    )
                                })?
                            },
                        };
                        let frame = json_frame(&response).ok_or_else(|| {
                            ServeFailure::Io("failed to encode local interaction".to_owned())
                        })?;
                        write_frame(&mut upstream_out, &frame).await.map_err(|error| {
                            ServeFailure::Io(format!(
                                "failed to answer local interaction: {error}"
                            ))
                        })?;
                    },
                    TransportAction::Cancel(request) => {
                        let response = interaction::cancelled_response(&request).ok_or_else(|| {
                            ServeFailure::Io(
                                "elicitation request did not carry a response id".to_owned(),
                            )
                        })?;
                        let frame = json_frame(&response).ok_or_else(|| {
                            ServeFailure::Io("failed to encode cancelled interaction".to_owned())
                        })?;
                        write_frame(&mut upstream_out, &frame).await.map_err(|error| {
                            ServeFailure::Io(format!(
                                "failed to answer local interaction: {error}"
                            ))
                        })?;
                    },
                }
                transport_frame.clear();
            },
            interrupt = next_interrupt(interrupt_rx.as_mut()), if interrupt_rx.is_some() => {
                let Some(signal) = interrupt else {
                    interrupt_rx = None;
                    continue;
                };
                return Err(terminate(
                    child,
                    downstream_in.take(),
                    ServeFailure::Interrupted(signal),
                )
                .await);
            },
        }
    }

    drop(downstream_in);
    let status = child.wait().await.map_err(|error| {
        ServeFailure::Io(format!("failed waiting for bundled MCP transport: {error}"))
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(ServeFailure::Exit(status))
    }
}

async fn write_frame(
    output: &mut (impl tokio::io::AsyncWrite + Unpin),
    frame: &[u8],
) -> std::io::Result<()> {
    output.write_all(frame).await?;
    output.flush().await
}

async fn read_frame(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
    frame: &mut Vec<u8>,
) -> std::io::Result<usize> {
    frame.clear();
    reader.read_until(b'\n', frame).await
}

async fn next_interrupt(
    receiver: Option<&mut UnboundedReceiver<InterruptSignal>>,
) -> Option<InterruptSignal> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

async fn terminate(
    mut child: tokio::process::Child,
    child_stdin: Option<tokio::process::ChildStdin>,
    failure: ServeFailure,
) -> ServeFailure {
    drop(child_stdin);
    if let Err(error) = child.start_kill() {
        return ServeFailure::Io(format!(
            "failed to terminate bundled MCP transport after failure: {error}"
        ));
    }
    if let Err(error) = child.wait().await {
        return ServeFailure::Io(format!(
            "failed waiting for terminated bundled MCP transport: {error}"
        ));
    }
    failure
}

fn transport_exit_code(status: ExitStatus) -> ExitCode {
    if status.success() {
        return ExitCode::SUCCESS;
    }
    if let Some(code) = status.code() {
        return ExitCode::from(code.clamp(1, i32::from(u8::MAX)) as u8);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        status
            .signal()
            .map(|signal| ExitCode::from(u8::try_from(128 + signal).unwrap_or(u8::MAX)))
            .unwrap_or(ExitCode::FAILURE)
    }
    #[cfg(not(unix))]
    ExitCode::FAILURE
}

fn json_frame(value: &Value) -> Option<Vec<u8>> {
    let mut frame = serde_json::to_vec(value).ok()?;
    frame.push(b'\n');
    Some(frame)
}

fn prepare_client_message(
    frame: &[u8],
    mode: InteractionMode,
    client_supports_form: &mut bool,
) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(frame).ok()?;
    let mut value = serde_json::from_str::<Value>(text).ok()?;
    if value.get("method").and_then(Value::as_str) != Some("initialize") {
        return None;
    }
    *client_supports_form = supports_form_elicitation(&value);
    if matches!(mode, InteractionMode::Auto | InteractionMode::Native)
        && (mode == InteractionMode::Native || !*client_supports_form)
    {
        advertise_form_elicitation(&mut value);
    }
    Some(serde_json::to_vec(&value).unwrap_or_else(|_| frame.to_vec()))
}

enum TransportAction {
    Forward,
    Rewrite(Value),
    Present(Value),
    Cancel(Value),
}

fn transport_action(
    frame: &[u8],
    mode: InteractionMode,
    client_supports_form: bool,
) -> TransportAction {
    let Ok(text) = std::str::from_utf8(frame) else {
        return TransportAction::Forward;
    };
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return TransportAction::Forward;
    };
    let handling = elicitation_handling(&message, mode, client_supports_form);
    match handling {
        ElicitationHandling::Forward => {
            if message.pointer("/result/protocolVersion").is_some()
                && message.pointer("/result/capabilities").is_some()
            {
                TransportAction::Rewrite(message)
            } else {
                TransportAction::Forward
            }
        }
        ElicitationHandling::Present => TransportAction::Present(message),
        ElicitationHandling::Cancel => TransportAction::Cancel(message),
    }
}

fn supports_form_elicitation(initialize: &Value) -> bool {
    initialize
        .pointer("/params/capabilities/elicitation/form")
        .is_some_and(Value::is_object)
}

fn advertise_form_elicitation(initialize: &mut Value) {
    let Some(params) = initialize.get_mut("params").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(capabilities) = object_entry(params, "capabilities") else {
        return;
    };
    let Some(elicitation) = object_entry(capabilities, "elicitation") else {
        return;
    };
    elicitation
        .entry("form".to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
}

fn object_entry<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> Option<&'a mut Map<String, Value>> {
    let value = object
        .entry(key.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    value.as_object_mut()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElicitationHandling {
    Forward,
    Present,
    Cancel,
}

fn elicitation_handling(
    message: &Value,
    mode: InteractionMode,
    client_supports_form: bool,
) -> ElicitationHandling {
    if message.get("method").and_then(Value::as_str) != Some("elicitation/create") {
        return ElicitationHandling::Forward;
    }
    if mode == InteractionMode::Deny {
        return ElicitationHandling::Cancel;
    }
    if message
        .pointer("/params/mode")
        .and_then(Value::as_str)
        .unwrap_or("form")
        != "form"
    {
        return ElicitationHandling::Forward;
    }
    match mode {
        InteractionMode::Auto if !client_supports_form => ElicitationHandling::Present,
        InteractionMode::Native => ElicitationHandling::Present,
        InteractionMode::Auto | InteractionMode::Client => ElicitationHandling::Forward,
        InteractionMode::Deny => unreachable!("handled above"),
    }
}

fn rewrite_server_identity(message: &mut Value) {
    if message.pointer("/result/protocolVersion").is_none()
        || message.pointer("/result/capabilities").is_none()
    {
        return;
    }
    let Some(info) = message
        .pointer_mut("/result/serverInfo")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    info.insert("name".to_owned(), Value::String("unicity-aos".to_owned()));
    info.insert("title".to_owned(), Value::String("Unicity AOS".to_owned()));
    info.insert(
        "version".to_owned(),
        Value::String(env!("CARGO_PKG_VERSION").to_owned()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn initialize(capabilities: Value) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": capabilities,
                "clientInfo": { "name": "test", "version": "1" }
            }
        })
        .to_string()
    }

    #[test]
    fn auto_advertises_form_only_when_client_cannot_present_it() {
        let mut supported = false;
        let forwarded = prepare_client_message(
            initialize(json!({ "roots": {} })).as_bytes(),
            InteractionMode::Auto,
            &mut supported,
        )
        .expect("initialize is transformed");
        let forwarded: Value = serde_json::from_slice(&forwarded).expect("json");
        assert!(!supported);
        assert!(
            forwarded
                .pointer("/params/capabilities/elicitation/form")
                .is_some()
        );

        let mut supported = false;
        let forwarded = prepare_client_message(
            initialize(json!({ "elicitation": { "form": {} } })).as_bytes(),
            InteractionMode::Auto,
            &mut supported,
        )
        .expect("initialize is transformed");
        let forwarded: Value = serde_json::from_slice(&forwarded).expect("json");
        assert!(supported);
        assert!(
            forwarded
                .pointer("/params/capabilities/elicitation/form")
                .is_some()
        );
    }

    #[test]
    fn client_and_deny_modes_never_invent_capabilities() {
        for mode in [InteractionMode::Client, InteractionMode::Deny] {
            let mut supported = false;
            let forwarded =
                prepare_client_message(initialize(json!({})).as_bytes(), mode, &mut supported)
                    .expect("initialize is transformed");
            let forwarded: Value = serde_json::from_slice(&forwarded).expect("json");
            assert!(
                forwarded
                    .pointer("/params/capabilities/elicitation/form")
                    .is_none()
            );
        }
    }

    #[test]
    fn malformed_initialize_capabilities_are_not_rewritten() {
        let malformed = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": "not-an-object" }
        })
        .to_string();
        let mut supported = false;
        let forwarded =
            prepare_client_message(malformed.as_bytes(), InteractionMode::Auto, &mut supported)
                .expect("initialize is transformed");
        let forwarded: Value = serde_json::from_slice(&forwarded).expect("json");
        assert_eq!(forwarded["params"]["capabilities"], "not-an-object");
        assert!(!supported);
    }

    #[test]
    fn auto_intercepts_only_when_the_client_lacks_form_support() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "elicitation/create",
            "params": { "mode": "form" }
        });
        assert_eq!(
            elicitation_handling(&request, InteractionMode::Auto, false),
            ElicitationHandling::Present
        );
        assert_eq!(
            elicitation_handling(&request, InteractionMode::Auto, true),
            ElicitationHandling::Forward
        );
        assert_eq!(
            elicitation_handling(&request, InteractionMode::Native, true),
            ElicitationHandling::Present
        );
    }

    #[test]
    fn url_elicitation_is_never_intercepted_as_a_local_form() {
        let request = json!({
            "method": "elicitation/create",
            "params": { "mode": "url" }
        });
        assert_eq!(
            elicitation_handling(&request, InteractionMode::Native, false),
            ElicitationHandling::Forward
        );
    }

    #[test]
    fn deny_mode_cancels_form_and_url_elicitation() {
        for request in [
            json!({ "method": "elicitation/create", "params": { "mode": "form" } }),
            json!({ "method": "elicitation/create", "params": { "mode": "url" } }),
        ] {
            assert_eq!(
                elicitation_handling(&request, InteractionMode::Deny, true),
                ElicitationHandling::Cancel
            );
        }
    }

    #[test]
    fn initialize_response_is_product_branded() {
        let mut response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "serverInfo": { "name": "astrid", "version": "0.10.4" }
            }
        });
        rewrite_server_identity(&mut response);
        assert_eq!(response["result"]["serverInfo"]["name"], "unicity-aos");
        assert_eq!(response["result"]["serverInfo"]["title"], "Unicity AOS");
    }
}
