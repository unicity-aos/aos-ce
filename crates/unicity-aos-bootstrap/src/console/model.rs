//! Transient presentation state. Answers never enter history or debug output.
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use zeroize::Zeroizing;

pub(super) const CAPACITY: usize = 8;
pub(super) const MAX_VALUE: usize = 4096;

pub(super) enum Kind {
    Approval(Vec<String>),
    Text,
    Secret,
    Select(Vec<String>),
    Array,
}

pub(super) enum Destination {
    Approval(UnixStream),
    Private { stream: UnixStream, generation: u64 },
    Preview,
}

pub(super) struct Request {
    pub id: String,
    pub title: String,
    pub principal: String,
    pub message: String,
    pub detail: String,
    pub kind: Kind,
    pub deadline: Instant,
    pub destination: Destination,
}

impl Request {
    pub fn private(&self) -> bool {
        matches!(self.kind, Kind::Secret)
    }

    pub fn options(&self) -> Vec<String> {
        match &self.kind {
            Kind::Approval(options) => options.clone(),
            Kind::Select(options) => options.clone(),
            _ => vec!["Send securely".to_owned()],
        }
    }

    pub fn send(&mut self, selection: Option<usize>, input: &str) -> io::Result<()> {
        let response = match &self.destination {
            Destination::Approval(_) => {
                if selection.is_some_and(|i| i >= self.options().len()) {
                    return Err(io::Error::other("invalid selection"));
                }
                json!({"version":1,"id":self.id,"selected":selection})
            }
            Destination::Private { .. } | Destination::Preview => {
                let mut value = Value::Null;
                let mut values = Value::Null;
                if let Some(index) = selection {
                    if input.len() > MAX_VALUE {
                        return Err(io::Error::other("input too long"));
                    }
                    match &self.kind {
                        Kind::Secret if input.is_empty() => {
                            return Err(io::Error::other("enter a value or cancel"));
                        }
                        Kind::Select(options) => {
                            value = json!(
                                options
                                    .get(index)
                                    .ok_or_else(|| io::Error::other("invalid choice"))?
                            )
                        }
                        Kind::Array => {
                            let lines: Vec<_> = input.lines().collect();
                            if lines.len() > 64 {
                                return Err(io::Error::other("at most 64 items"));
                            }
                            values = json!(lines);
                        }
                        _ => value = json!(input),
                    }
                }
                json!({"topic":"astrid.v1.private.elicit.reply","source_id":"00000000-0000-0000-0000-000000000000", "payload":{"type":"elicit_response","request_id":self.id,"value":value,"values":values}})
            }
        };
        let bytes = Zeroizing::new(serde_json::to_vec(&response)?);
        match &mut self.destination {
            Destination::Approval(stream) => {
                stream.write_all(&bytes)?;
                stream.write_all(b"\n")?;
                stream.shutdown(Shutdown::Both)?;
            }
            Destination::Private { stream, .. } => {
                stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
                stream.write_all(&bytes)?;
            }
            Destination::Preview => {}
        }
        Ok(())
    }
}

/// Prevent terminal escapes, bidi controls, and embedded control sequences from
/// turning untrusted descriptions into terminal instructions.
pub(super) fn display(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

pub(super) fn preview() -> Vec<Request> {
    vec![
        Request { id:"preview-approval".into(), title:"aos-http".into(), principal:"codex-code".into(),
            message:"Connect to api.github.com".into(), detail:"Read pull-request status for your workspace.\nScope: network access to api.github.com\nThese choices are preview data.".into(),
            kind:Kind::Approval(vec!["Allow once".into(),"Until runtime restart".into(),"Deny".into()]),
            deadline:Instant::now()+Duration::from_secs(3600), destination:Destination::Preview },
        Request { id:"preview-secret".into(), title:"aos-http".into(), principal:"codex-code".into(),
            message:"Enter an API token".into(), detail:"Private input · sent directly to the runtime\nNever included in the agent conversation.".into(), kind:Kind::Secret,
            deadline:Instant::now()+Duration::from_secs(3600), destination:Destination::Preview },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_controls_cannot_execute_or_reorder_labels() {
        assert_eq!(display("safe\x1b[2J\r\u{202e}text"), "safe[2Jtext");
    }
    #[test]
    fn empty_secrets_cannot_be_submitted_but_can_be_cancelled() {
        let mut requests = preview();
        let secret = &mut requests[1];
        assert!(secret.send(Some(0), "").is_err());
        assert!(secret.send(None, "").is_ok());
    }
}
