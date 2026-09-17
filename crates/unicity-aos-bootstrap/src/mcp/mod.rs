//! Product-owned MCP endpoint and local interaction bridge.
//!
//! The pinned runtime's MCP shim remains a compatibility transport for this
//! release. AOS owns the externally visible command, server identity, and
//! interaction policy. That lets hosts without MCP form elicitation use a
//! trusted local decision surface without weakening or forking the runtime.

#[cfg(test)]
mod framing_tests;
mod interaction;
mod mrtr;
#[cfg(test)]
mod serve_tests;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{ExitCode, ExitStatus, Stdio};

use clap::{Args, ValueEnum};
use serde_json::{Map, Value, json};
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
    /// Unix socket for native tray presentation. Requires `--interaction native`.
    #[arg(long = "interaction-socket", value_name = "PATH")]
    interaction_socket: Option<PathBuf>,
    /// Native tray prompt deadline in seconds. Distinct from `--request-timeout`.
    #[arg(
        long = "interaction-timeout",
        value_name = "SECONDS",
        default_value_t = interaction::DEFAULT_INTERACTION_TIMEOUT_SECONDS,
        value_parser = interaction::parse_interaction_timeout
    )]
    interaction_timeout: u32,
    /// Maximum tracked in-flight `tools/call` invocations for native MRTR.
    #[arg(
        long = "max-in-flight-calls",
        value_name = "COUNT",
        default_value_t = mrtr::DEFAULT_MAX_IN_FLIGHT_CALLS,
        value_parser = mrtr::parse_max_in_flight_calls
    )]
    max_in_flight_calls: usize,
    /// Maximum distinct consumed input rounds retained per tracked `tools/call`.
    #[arg(
        long = "max-input-rounds",
        value_name = "COUNT",
        default_value_t = mrtr::DEFAULT_MAX_INPUT_ROUNDS,
        value_parser = mrtr::parse_max_input_rounds
    )]
    max_input_rounds: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeSurface {
    Platform,
    #[cfg(unix)]
    Socket(PathBuf),
}

fn native_surface(args: &ServeArgs) -> Result<NativeSurface, String> {
    match args.interaction_socket.as_ref() {
        None => Ok(NativeSurface::Platform),
        Some(_) if args.interaction != InteractionMode::Native => {
            Err("--interaction-socket requires --interaction native".to_owned())
        }
        Some(path) => {
            #[cfg(unix)]
            {
                Ok(NativeSurface::Socket(path.clone()))
            }
            #[cfg(not(unix))]
            {
                let _ = path;
                Err("--interaction-socket is only supported on Unix".to_owned())
            }
        }
    }
}

enum ServePresenter {
    Platform(interaction::NativePresenter),
    #[cfg(unix)]
    Socket(interaction::TrayPresenter),
}

impl interaction::Presenter for ServePresenter {
    fn present(
        &mut self,
        request: &interaction::InteractionRequest,
    ) -> Result<Option<usize>, interaction::InteractionError> {
        match self {
            Self::Platform(presenter) => presenter.present(request),
            #[cfg(unix)]
            Self::Socket(presenter) => presenter.present(request),
        }
    }
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
    if let Err(error) = native_surface(&args) {
        eprintln!("aos mcp serve: {error}");
        return ExitCode::FAILURE;
    }
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
    home.ensure_runtime_transport_available()
        .map_err(|error| ServeFailure::Io(format!("failed to prepare bundled runtime: {error}")))?;
    #[cfg(not(unix))]
    let _ = args.interaction_timeout;
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
    let mut presenter = match native_surface(args) {
        Ok(NativeSurface::Platform) => ServePresenter::Platform(interaction::NativePresenter),
        #[cfg(unix)]
        Ok(NativeSurface::Socket(path)) => ServePresenter::Socket(interaction::TrayPresenter::new(
            path,
            args.interaction_timeout,
        )),
        Err(error) => return Err(ServeFailure::Io(error)),
    };
    let mut mrtr = mrtr::NativeMrtr::with_limits(args.max_in_flight_calls, args.max_input_rounds);
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
                let rewritten;
                let outbound = match prepare_client_message(
                    &client_frame,
                    mode,
                    &mut client_supports_form,
                    &mut mrtr,
                ) {
                    UpstreamPrepare::Unchanged => client_frame.as_slice(),
                    UpstreamPrepare::Rewrite(frame) => {
                        rewritten = frame;
                        rewritten.as_slice()
                    }
                    UpstreamPrepare::Reply(frame) => {
                        write_frame(&mut upstream_out, &frame).await.map_err(|error| {
                            ServeFailure::Io(format!("failed to write MCP client: {error}"))
                        })?;
                        client_frame.clear();
                        continue;
                    }
                    UpstreamPrepare::Reject(error) => {
                        return Err(ServeFailure::Io(format!(
                            "refusing to forward tools/call: {error}"
                        )));
                    }
                };
                let transport_input = downstream_in.as_mut().ok_or_else(|| {
                    ServeFailure::Io("bundled MCP transport input is closed".to_owned())
                })?;
                write_frame(transport_input, outbound).await.map_err(|error| {
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
                match intercept_downstream(
                    &transport_frame,
                    mode,
                    client_supports_form,
                    &mut mrtr,
                    &mut presenter,
                ) {
                    DownstreamIntercept::Resume(resume) => {
                        let frame = json_frame(&resume).ok_or_else(|| {
                            ServeFailure::Io("failed to encode native input_required resume".to_owned())
                        })?;
                        let transport_input = downstream_in.as_mut().ok_or_else(|| {
                            ServeFailure::Io("bundled MCP transport input is closed".to_owned())
                        })?;
                        write_frame(transport_input, &frame).await.map_err(|error| {
                            ServeFailure::Io(format!(
                                "failed to resume native input_required: {error}"
                            ))
                        })?;
                        transport_frame.clear();
                        continue;
                    }
                    DownstreamIntercept::Swallow => {
                        transport_frame.clear();
                        continue;
                    }
                    DownstreamIntercept::HostError(response) => {
                        let frame = json_frame(&response).ok_or_else(|| {
                            ServeFailure::Io(
                                "failed to encode native input_required error".to_owned(),
                            )
                        })?;
                        write_frame(&mut upstream_out, &frame).await.map_err(|error| {
                            ServeFailure::Io(format!("failed to write MCP client: {error}"))
                        })?;
                        transport_frame.clear();
                        continue;
                    }
                    DownstreamIntercept::Exhausted {
                        response,
                        runtime_cancel,
                    } => {
                        let frame = json_frame(&response).ok_or_else(|| {
                            ServeFailure::Io(
                                "failed to encode native input_required error".to_owned(),
                            )
                        })?;
                        write_frame(&mut upstream_out, &frame).await.map_err(|error| {
                            ServeFailure::Io(format!("failed to write MCP client: {error}"))
                        })?;
                        let cancel = json_frame(&runtime_cancel).ok_or_else(|| {
                            ServeFailure::Io(
                                "failed to encode exhausted-call cancellation".to_owned(),
                            )
                        })?;
                        let transport_input = downstream_in.as_mut().ok_or_else(|| {
                            ServeFailure::Io("bundled MCP transport input is closed".to_owned())
                        })?;
                        write_frame(transport_input, &cancel).await.map_err(|error| {
                            ServeFailure::Io(format!(
                                "failed to cancel exhausted native tools/call: {error}"
                            ))
                        })?;
                        transport_frame.clear();
                        continue;
                    }
                    DownstreamIntercept::None => {}
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
                    TransportAction::Rewrite(message) => {
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
                        // The runtime issued this request. The host must only see
                        // the resulting tool response, not our JSON-RPC answer.
                        let transport_input = downstream_in.as_mut().ok_or_else(|| {
                            ServeFailure::Io("bundled MCP transport input is closed".to_owned())
                        })?;
                        write_frame(transport_input, &frame).await.map_err(|error| {
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
                        let transport_input = downstream_in.as_mut().ok_or_else(|| {
                            ServeFailure::Io("bundled MCP transport input is closed".to_owned())
                        })?;
                        write_frame(transport_input, &frame).await.map_err(|error| {
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
    // The other select branch can win after read_until appends a prefix.
    // Preserve that prefix across cancellation; the caller clears only after
    // handling a complete frame. At EOF, retained bytes still form a final
    // frame rather than an empty-stream indication.
    reader.read_until(b'\n', frame).await?;
    Ok(frame.len())
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

fn jsonrpc_error_for(request: &Value, error: &mrtr::MrtrError) -> Option<Value> {
    Some(json!({
        "jsonrpc": request.get("jsonrpc").cloned().unwrap_or_else(|| {
            Value::String("2.0".to_owned())
        }),
        "id": request.get("id")?.clone(),
        "error": {
            "code": -32603,
            "message": error.to_string(),
        }
    }))
}

#[derive(Debug, PartialEq, Eq)]
enum UpstreamPrepare {
    Unchanged,
    Rewrite(Vec<u8>),
    Reply(Vec<u8>),
    Reject(mrtr::MrtrError),
}

fn prepare_client_message(
    frame: &[u8],
    mode: InteractionMode,
    client_supports_form: &mut bool,
    mrtr: &mut mrtr::NativeMrtr,
) -> UpstreamPrepare {
    let Ok(text) = std::str::from_utf8(frame) else {
        return UpstreamPrepare::Unchanged;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(text) else {
        return UpstreamPrepare::Unchanged;
    };
    if value.get("method").and_then(Value::as_str) == Some("initialize") {
        *client_supports_form = supports_form_elicitation(&value);
        if matches!(mode, InteractionMode::Auto | InteractionMode::Native)
            && (mode == InteractionMode::Native || !*client_supports_form)
            && advertise_form_elicitation(&mut value)
        {
            return UpstreamPrepare::Rewrite(json_frame(&value).unwrap_or_else(|| frame.to_vec()));
        }
        return UpstreamPrepare::Unchanged;
    }
    if !intercepts_local_input(mode, *client_supports_form) {
        return UpstreamPrepare::Unchanged;
    }
    if let Some(id) = mrtr::cancelled_request_id(&value) {
        mrtr.forget(id);
        return UpstreamPrepare::Unchanged;
    }
    if value.get("method").and_then(Value::as_str) != Some("tools/call") {
        return UpstreamPrepare::Unchanged;
    }
    let advertised = matches!(mode, InteractionMode::Auto | InteractionMode::Native)
        && advertise_request_form_meta(&mut value);
    if let Err(error) = mrtr.record(&value) {
        if error == mrtr::MrtrError::TooManyInFlight
            && let Some(response) = jsonrpc_error_for(&value, &error)
            && let Some(frame) = json_frame(&response)
        {
            return UpstreamPrepare::Reply(frame);
        }
        return UpstreamPrepare::Reject(error);
    }
    if advertised {
        UpstreamPrepare::Rewrite(json_frame(&value).unwrap_or_else(|| frame.to_vec()))
    } else {
        UpstreamPrepare::Unchanged
    }
}

fn intercepts_local_input(mode: InteractionMode, client_supports_form: bool) -> bool {
    match mode {
        InteractionMode::Native | InteractionMode::Deny => true,
        InteractionMode::Auto => !client_supports_form,
        InteractionMode::Client => false,
    }
}

enum DownstreamIntercept {
    None,
    Resume(Value),
    Swallow,
    HostError(Value),
    Exhausted {
        response: Value,
        runtime_cancel: Value,
    },
}

fn intercept_downstream(
    frame: &[u8],
    mode: InteractionMode,
    client_supports_form: bool,
    mrtr: &mut mrtr::NativeMrtr,
    presenter: &mut dyn interaction::Presenter,
) -> DownstreamIntercept {
    if !intercepts_local_input(mode, client_supports_form) {
        return DownstreamIntercept::None;
    }
    let Ok(text) = std::str::from_utf8(frame) else {
        return DownstreamIntercept::None;
    };
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return DownstreamIntercept::None;
    };
    if mrtr::is_input_required_result(&message) {
        let resume = match mode {
            InteractionMode::Deny => mrtr.decline(&message),
            InteractionMode::Native | InteractionMode::Auto | InteractionMode::Client => {
                mrtr.decide(&message, presenter)
            }
        };
        return match resume {
            Ok(resume) => DownstreamIntercept::Resume(resume),
            Err(mrtr::MrtrError::UnknownId | mrtr::MrtrError::AlreadySettled) => {
                DownstreamIntercept::Swallow
            }
            Err(mrtr::MrtrError::TooManyRounds) => {
                let Some(id) = message.get("id") else {
                    return DownstreamIntercept::Swallow;
                };
                let Some(response) = jsonrpc_error_for(&message, &mrtr::MrtrError::TooManyRounds)
                else {
                    return DownstreamIntercept::Swallow;
                };
                mrtr.complete(id);
                DownstreamIntercept::Exhausted {
                    response,
                    runtime_cancel: json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/cancelled",
                        "params": { "requestId": id },
                    }),
                }
            }
            Err(error) => match message.get("id") {
                Some(id) => {
                    mrtr.complete(id);
                    DownstreamIntercept::HostError(json!({
                        "jsonrpc": message.get("jsonrpc").cloned().unwrap_or_else(|| {
                            Value::String("2.0".to_owned())
                        }),
                        "id": id,
                        "error": {
                            "code": -32603,
                            "message": error.to_string(),
                        }
                    }))
                }
                None => DownstreamIntercept::Swallow,
            },
        };
    }
    if message.get("method").is_none()
        && let Some(id) = message.get("id")
    {
        mrtr.complete(id);
    }
    DownstreamIntercept::None
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
    let Ok(mut message) = serde_json::from_str::<Value>(text) else {
        return TransportAction::Forward;
    };
    let handling = elicitation_handling(&message, mode, client_supports_form);
    match handling {
        ElicitationHandling::Forward => {
            if rewrite_server_identity(&mut message) {
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

fn advertise_form_elicitation(initialize: &mut Value) -> bool {
    let Some(params) = initialize.get_mut("params").and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(capabilities) = object_entry(params, "capabilities") else {
        return false;
    };
    advertise_form_on_capabilities(capabilities)
}

fn advertise_request_form_meta(message: &mut Value) -> bool {
    let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(meta) = params.get_mut("_meta").and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(capabilities) = object_entry(meta, "io.modelcontextprotocol/clientCapabilities")
    else {
        return false;
    };
    advertise_form_on_capabilities(capabilities)
}

fn advertise_form_on_capabilities(capabilities: &mut Map<String, Value>) -> bool {
    let Some(elicitation) = object_entry(capabilities, "elicitation") else {
        return false;
    };
    if elicitation.contains_key("form") {
        return false;
    }
    elicitation.insert("form".to_owned(), Value::Object(Map::new()));
    true
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

fn rewrite_server_identity(message: &mut Value) -> bool {
    if message.pointer("/result/protocolVersion").is_none()
        || message.pointer("/result/capabilities").is_none()
    {
        return false;
    }
    let Some(info) = message
        .pointer_mut("/result/serverInfo")
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    let mut changed = false;
    let name = Value::String("unicity-aos".to_owned());
    if info.get("name") != Some(&name) {
        info.insert("name".to_owned(), name);
        changed = true;
    }
    let title = Value::String("Unicity AOS".to_owned());
    if info.get("title") != Some(&title) {
        info.insert("title".to_owned(), title);
        changed = true;
    }
    let version = Value::String(env!("CARGO_PKG_VERSION").to_owned());
    if info.get("version") != Some(&version) {
        info.insert("version".to_owned(), version);
        changed = true;
    }
    changed
}
