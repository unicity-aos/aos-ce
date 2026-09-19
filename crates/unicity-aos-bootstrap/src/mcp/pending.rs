//! Session-local, read-only observation of native approval continuations.
//! Human decisions never come from status-tool arguments.
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{interaction, mrtr};

pub(super) const STATUS_TOOL: &str = "aos_approval_status";
const PREFIX: &str = "aos-local-approval/";
const MAX_RESULT_BYTES: usize = 1_048_576;

type PresenterFactory = Box<dyn Fn() -> Box<dyn interaction::Presenter + Send> + Send + Sync>;

struct Entry {
    tool: String,
    host_id: Value,
    deferred: bool,
    state: &'static str,
    result: Option<Value>,
    deadline: Instant,
    context: Option<mrtr::NativeMrtr>,
    waiting: bool,
    observed: bool,
    worker_outstanding: bool,
}

struct Decision {
    id: String,
    context: mrtr::NativeMrtr,
    resume: Result<Value, mrtr::MrtrError>,
}

pub(super) struct PendingApprovals {
    guidance: String,
    entries: HashMap<String, Entry>,
    lists: Vec<(Value, bool)>,
    factory: PresenterFactory,
    sender: Sender<Decision>,
    receiver: Receiver<Decision>,
    limit: usize,
    rounds: usize,
    timeout: Duration,
}

pub(super) enum Upstream {
    Forward(Value),
    Reply(Value),
}

pub(super) enum Downstream {
    Forward(Value),
    Consumed,
}

fn result(id: Value, value: Value, error: bool) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{
        "content":[{"type":"text","text":value.to_string()}],
        "structuredContent":value,"isError":error,"resultType":"complete"
    }})
}

fn fault(id: Value, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":message}})
}

impl PendingApprovals {
    pub(super) fn from_args(args: &super::ServeArgs, home: &std::path::Path) -> Option<Self> {
        #[cfg(unix)]
        if let Ok(super::NativeSurface::Socket(path)) = super::native_surface(args) {
            let seconds = args.interaction_timeout.min(240);
            let guidance = approval_guidance(&path, home);
            let mut pending = Self::new(
                Box::new(move || Box::new(interaction::TrayPresenter::new(path.clone(), seconds))),
                args.max_in_flight_calls,
                args.max_input_rounds,
                Duration::from_secs(u64::from(seconds)),
            );
            pending.guidance = guidance;
            return Some(pending);
        }
        let _ = (args, home);
        None
    }

    pub(super) fn new(
        factory: PresenterFactory,
        limit: usize,
        rounds: usize,
        timeout: Duration,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            guidance: "Review the request in the configured AOS approval surface on the machine running this MCP server.".into(),
            entries: HashMap::new(),
            lists: Vec::new(),
            factory,
            sender,
            receiver,
            limit,
            rounds,
            timeout,
        }
    }

    pub(super) fn upstream(&mut self, mut message: Value) -> Upstream {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        // Runtime correlation ids belong exclusively to this adapter.
        if id.as_str().is_some_and(|id| id.starts_with(PREFIX)) {
            return Upstream::Reply(fault(id, "reserved request id"));
        }
        match message.get("method").and_then(Value::as_str) {
            Some("tools/list") => {
                if self.lists.len() >= self.limit {
                    return Upstream::Reply(fault(id, "too many pending tool listings"));
                }
                let first_page = message.pointer("/params/cursor").is_none_or(Value::is_null);
                self.lists.push((id, first_page));
            }
            Some("tools/call")
                if message.pointer("/params/name").and_then(Value::as_str) == Some(STATUS_TOOL) =>
            {
                return Upstream::Reply(self.status(id, message.pointer("/params/arguments")));
            }
            Some("tools/call") => {
                // This adapter, not the coding host, consumes and resumes the
                // runtime's input_required contract. Advertise its capabilities
                // only on this server-facing connection. Hosts still receive
                // ordinary tool results and never opaque continuation state.
                let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) else {
                    return Upstream::Reply(fault(id, "invalid tool invocation"));
                };
                let meta = params.entry("_meta").or_insert_with(|| json!({}));
                let Some(meta) = meta.as_object_mut() else {
                    return Upstream::Reply(fault(id, "invalid tool metadata"));
                };
                meta.insert(
                    "io.modelcontextprotocol/protocolVersion".into(),
                    json!("2026-07-28"),
                );
                let Some(capabilities) =
                    super::object_entry(meta, "io.modelcontextprotocol/clientCapabilities")
                else {
                    return Upstream::Reply(fault(id, "invalid client capabilities"));
                };
                if capabilities
                    .get("elicitation")
                    .is_some_and(|value| !value.is_object())
                {
                    return Upstream::Reply(fault(id, "invalid elicitation capabilities"));
                }
                super::advertise_form_on_capabilities(capabilities);
                if self.entries.len() >= self.limit {
                    let retired = self
                        .entries
                        .iter()
                        .filter(|(_, e)| {
                            e.observed
                                && !e.worker_outstanding
                                && matches!(e.state, "completed" | "expired" | "unavailable")
                        })
                        .min_by_key(|(_, e)| e.deadline)
                        .map(|(id, _)| id.clone());
                    if let Some(id) = retired {
                        self.entries.remove(&id);
                    }
                }
                if self.entries.len() >= self.limit {
                    return Upstream::Reply(fault(
                        id,
                        "approval session capacity reached; no new operation was started",
                    ));
                }
                if self
                    .entries
                    .values()
                    .any(|entry| !entry.deferred && entry.host_id == id)
                {
                    return Upstream::Reply(fault(id, "tool request id already in flight"));
                }
                if !(id.is_string() || id.as_i64().is_some()) {
                    return Upstream::Reply(fault(
                        id,
                        "tool request id must be a string or integer",
                    ));
                }
                let private_id = format!("{PREFIX}{}", uuid::Uuid::new_v4());
                message["id"] = json!(private_id);
                let mut context = mrtr::NativeMrtr::with_limits(1, self.rounds);
                if context.record(&message).is_err() {
                    return Upstream::Reply(fault(id, "invalid tool invocation"));
                }
                self.entries.insert(
                    private_id,
                    Entry {
                        tool: message
                            .pointer("/params/name")
                            .and_then(Value::as_str)
                            .unwrap_or("operation")
                            .to_owned(),
                        host_id: id,
                        deferred: false,
                        state: "running",
                        result: None,
                        deadline: Instant::now() + self.timeout,
                        context: Some(context),
                        waiting: false,
                        observed: false,
                        worker_outstanding: false,
                    },
                );
            }
            Some("notifications/cancelled") => {
                if let Some(host_id) = message.pointer("/params/requestId") {
                    let key = self
                        .entries
                        .iter()
                        .find(|(_, e)| !e.deferred && &e.host_id == host_id)
                        .map(|(k, _)| k.clone());
                    if let Some(key) = key {
                        self.entries.remove(&key);
                        message["params"]["requestId"] = json!(key);
                    }
                }
            }
            _ => {}
        }
        Upstream::Forward(message)
    }

    fn status(&mut self, id: Value, arguments: Option<&Value>) -> Value {
        let Some(args) = arguments.and_then(Value::as_object) else {
            return fault(id, "provide only approval_id");
        };
        if args.len() != 1 {
            return fault(
                id,
                "status accepts only approval_id; decisions and secrets are never accepted",
            );
        }
        let Some(key) = args.get("approval_id").and_then(Value::as_str) else {
            return fault(id, "approval_id must be a string");
        };
        let Some(entry) = self.entries.get_mut(key).filter(|e| e.deferred) else {
            return result(
                id,
                json!({"status":"unknown","message":"No approval with this id exists in this MCP session. Do not repeat the original action automatically."}),
                true,
            );
        };
        entry.observed = true;
        result(
            id,
            json!({"approval_id":key,"status":entry.state,"outcome":entry.result,"tool":entry.tool,
            "message": match entry.state {
                "awaiting_approval" => &self.guidance,
                "running" => "The human decision has been sent to the runtime. The operation's outcome is not yet known. Do not repeat the original action.",
                "expired" => "The local approval deadline expired. No late decision will resume this operation.",
                "unavailable" => "The local approval could not be completed. No approval was synthesized.",
                _ => "Report the original runtime outcome below; completion does not itself mean the operation succeeded."
            }}),
            false,
        )
    }

    pub(super) fn downstream(&mut self, mut message: Value) -> Downstream {
        if message.get("method").is_some() {
            return Downstream::Forward(message);
        }
        if let Some(id) = message.get("id").cloned()
            && let Some(index) = self.lists.iter().position(|(x, _)| x == &id)
        {
            let (_, first_page) = self.lists.remove(index);
            if let Some(tools) = message
                .pointer_mut("/result/tools")
                .and_then(Value::as_array_mut)
            {
                if tools
                    .iter()
                    .any(|t| t.get("name").and_then(Value::as_str) == Some(STATUS_TOOL))
                {
                    return Downstream::Forward(fault(
                        id.clone(),
                        "runtime tool collides with AOS approval status",
                    ));
                }
                for tool in tools.iter_mut() {
                    if let Some(schema) = tool.get_mut("outputSchema") {
                        *schema = json!({"type":"object","anyOf":[schema.clone(),
                            {"type":"object","properties":{"status":{"enum":["awaiting_approval","unavailable"]},"message":{"type":"string"}},"required":["status","message"]}]});
                    }
                }
                if first_page {
                    tools.push(json!({"name":STATUS_TOOL,"description":"Read the outcome of an AOS pending approval in this session. Cannot approve, supply secrets, or repeat an operation. Tell the user when status is awaiting_approval; check again after their response, not continuously.","inputSchema":{"type":"object","properties":{"approval_id":{"type":"string"}},"required":["approval_id"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true}}));
                }
            }
            return Downstream::Forward(message);
        }
        let Some(key) = message
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with(PREFIX))
            .map(str::to_owned)
        else {
            return Downstream::Forward(message);
        };
        let Some(entry) = self.entries.get_mut(&key) else {
            return Downstream::Consumed;
        };
        if entry.waiting
            || entry.result.is_some()
            || matches!(entry.state, "expired" | "unavailable")
        {
            return Downstream::Consumed;
        }
        if mrtr::is_input_required_result(&message) {
            let Some(context) = entry.context.take() else {
                return Downstream::Consumed;
            };
            // Validate the round and every form before claiming to await a human.
            if context.validate_presentation(&message).is_err() {
                entry.context = Some(context);
                entry.state = "unavailable";
                if !entry.deferred {
                    let id = entry.host_id.clone();
                    self.entries.remove(&key);
                    return Downstream::Forward(result(
                        id,
                        json!({"status":"unavailable","message":"AOS could not present this approval safely. No consent was granted."}),
                        true,
                    ));
                }
                return Downstream::Consumed;
            }
            entry.waiting = true;
            entry.worker_outstanding = true;
            entry.state = "awaiting_approval";
            entry.deadline = Instant::now() + self.timeout;
            let first = !entry.deferred;
            entry.deferred = true;
            let host_id = entry.host_id.clone();
            let sender = self.sender.clone();
            let job_id = key.clone();
            let mut presenter = (self.factory)();
            let spawn = std::thread::Builder::new()
                .name("aos-human-approval".into())
                .spawn(move || {
                    let mut context = context;
                    let resume = context.decide(&message, presenter.as_mut());
                    let _ = sender.send(Decision {
                        id: job_id,
                        context,
                        resume,
                    });
                });
            if spawn.is_err() {
                entry.waiting = false;
                entry.worker_outstanding = false;
                entry.state = "unavailable";
            }
            if first {
                return Downstream::Forward(result(
                    host_id,
                    json!({"status":entry.state,"approval_id":key,"status_tool":STATUS_TOOL,"tool":entry.tool,
                    "message":if entry.state == "unavailable" { "AOS could not start the approval presenter. No approval was granted." } else { &self.guidance }}),
                    false,
                ));
            }
            return Downstream::Consumed;
        }
        if !entry.deferred {
            message["id"] = entry.host_id.clone();
            self.entries.remove(&key);
            return Downstream::Forward(message);
        }
        entry.state = "completed";
        entry.observed = false;
        // Preserve success/error exactly; "completed" never means "approved".
        entry.result = Some(if message.to_string().len() <= MAX_RESULT_BYTES {
            json!({"result":message.get("result"),"error":message.get("error")})
        } else {
            json!({"error":"Runtime outcome exceeds the retained size limit. Do not repeat the action; inspect runtime records."})
        });
        entry.context = None;
        Downstream::Consumed
    }

    pub(super) fn decisions(&mut self) -> Vec<Value> {
        let now = Instant::now();
        for entry in self
            .entries
            .values_mut()
            .filter(|e| e.waiting && e.deadline <= now)
        {
            entry.waiting = false;
            entry.state = "expired";
            entry.observed = false;
            entry.context = None;
        }
        let mut outbound = Vec::new();
        while let Ok(decision) = self.receiver.try_recv() {
            let Some(entry) = self.entries.get_mut(&decision.id) else {
                continue;
            };
            entry.worker_outstanding = false;
            if !entry.waiting || entry.deadline <= now {
                continue;
            }
            entry.waiting = false;
            match decision.resume {
                Ok(resume) => {
                    entry.context = Some(decision.context);
                    entry.state = "running";
                    outbound.push(resume);
                }
                Err(_) => {
                    entry.state = "unavailable";
                    entry.observed = false;
                    entry.context = None;
                }
            }
        }
        outbound
    }
}

/// Describe the selected surface, not the location of the coding client, which
/// may be remote. Never guess an SSH destination or expose operation arguments.
fn approval_guidance(socket: &std::path::Path, home: &std::path::Path) -> String {
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let console = socket == home.join("console/approval.sock");
    let mut message =
        String::from("Tell the user now that the named tool is waiting for their decision. ");
    if console {
        message.push_str("Review the request in the already-open AOS console on the machine running this MCP server. ");
    } else if cfg!(target_os = "macos") {
        message.push_str("Review the request in AOS Command Center on the Mac running this MCP server. AOS requests that the installed app open automatically; this does not confirm that it opened. ");
    } else {
        message.push_str("Review the request in the configured approval application on the machine running this MCP server. ");
    }
    if console && let Some(path) = home.to_str().filter(|s| !s.chars().any(char::is_control)) {
        let quoted = path.replace('\'', "'\\''");
        message.push_str(&format!("To open that environment's console, run: AOS_HOME='{quoted}' aos console. If a console is already open, return to it instead of starting a second one. "));
    }
    message.push_str("If that machine is remote, connect using your existing SSH connection first; do not run this on a different local AOS installation or invent an SSH address. Do not ask for approval or secrets in chat. Do not repeat the original tool call. Check aos_approval_status after the user responds, not in a tight polling loop. Completion reports the original operation's outcome, not automatic success.");
    message
}

#[cfg(test)]
mod guidance_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn console_guidance_names_exact_home_and_remote_boundary() {
        let home = Path::new("/nonexistent/Operator's AOS");
        let text = approval_guidance(&home.join("console/approval.sock"), home);
        assert!(text.contains("AOS_HOME='/nonexistent/Operator'\\''s AOS' aos console"));
        assert!(text.contains("already-open AOS console"));
        assert!(text.contains("existing SSH connection first"));
        assert!(text.contains("Do not ask for approval or secrets in chat"));
        assert!(!text.contains("Command Center"));
    }

    #[test]
    fn custom_presenter_does_not_direct_user_to_unrelated_console() {
        let text = approval_guidance(Path::new("/private/presenter.sock"), Path::new("/aos"));
        assert!(!text.contains("aos console"));
        assert!(!text.contains("AOS_HOME="));
        if cfg!(target_os = "macos") {
            assert!(text.contains("does not confirm that it opened"));
        }
    }
}
