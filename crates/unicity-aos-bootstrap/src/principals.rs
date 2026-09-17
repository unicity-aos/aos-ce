//! Owned-principal discovery. A returned row is not permission to act,
//! approve, or enter secrets.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::process::Output;

use astrid_core::PrincipalId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct RuntimeAgent {
    principal: String,
    #[serde(default)]
    enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OwnedPrincipal {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OwnedDiscovery {
    pub scope: String,
    pub authority: String,
    pub principals: Vec<OwnedPrincipal>,
}

impl OwnedDiscovery {
    fn owned(principals: Vec<OwnedPrincipal>) -> Self {
        Self {
            scope: "owned".to_owned(),
            authority: "discovery".to_owned(),
            principals,
        }
    }
}

pub(crate) const MINE_UNSUPPORTED_MESSAGE: &str =
    "owned-principal discovery is not supported by this runtime";

pub(crate) fn runtime_discovery_args(principal: &PrincipalId) -> Vec<OsString> {
    vec![
        OsString::from("--principal"),
        OsString::from(principal.as_str()),
        OsString::from("agent"),
        OsString::from("list"),
        OsString::from("--mine"),
        OsString::from("--format"),
        OsString::from("json"),
    ]
}

pub(crate) fn document_from_runtime_json(bytes: &[u8]) -> Result<OwnedDiscovery, String> {
    let agents: Vec<RuntimeAgent> = serde_json::from_slice(bytes)
        .map_err(|_| "runtime did not return an owned-principal directory".to_owned())?;
    let mut seen = BTreeSet::new();
    let mut principals = Vec::with_capacity(agents.len());
    for agent in agents {
        let id = PrincipalId::new(agent.principal)
            .map_err(|error| format!("invalid discovered principal: {error}"))?;
        let name = id.to_string();
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate discovered principal: {name}"));
        }
        principals.push(OwnedPrincipal {
            id: name,
            enabled: agent.enabled,
        });
    }
    Ok(OwnedDiscovery::owned(principals))
}

pub(crate) fn runtime_discovery_unsupported(output: &Output) -> bool {
    if output.status.success() {
        return false;
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    )
    .to_ascii_lowercase();
    let mentions_mine = text.contains("--mine");
    let unknown = text.contains("unexpected argument")
        || text.contains("unknown argument")
        || text.contains("unrecognized")
        || text.contains("invalid option")
        || text.contains("wasn't expected")
        || text.contains("was not expected");
    mentions_mine && unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_args_always_include_mine_and_never_a_global_list() {
        let args = runtime_discovery_args(&PrincipalId::default());
        let text: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            text,
            [
                "--principal",
                "default",
                "agent",
                "list",
                "--mine",
                "--format",
                "json"
            ]
        );
        assert!(text.contains(&"--mine".to_owned()));
        assert_ne!(text, ["agent", "list", "--format", "json"]);
    }

    #[test]
    fn empty_runtime_directory_is_owned_not_an_error() {
        let document = document_from_runtime_json(b"[]").expect("empty directory");
        assert_eq!(document.scope, "owned");
        assert_eq!(document.authority, "discovery");
        assert!(document.principals.is_empty());
    }

    #[test]
    fn runtime_records_are_mapped_without_grants_or_groups() {
        let document = document_from_runtime_json(
            br#"[{"principal":"alice","enabled":true,"groups":["agent"],"grants":["*"]},{"principal":"default","enabled":false}]"#,
        )
        .expect("owned directory");
        assert_eq!(
            document.principals,
            [
                OwnedPrincipal {
                    id: "alice".to_owned(),
                    enabled: true,
                },
                OwnedPrincipal {
                    id: "default".to_owned(),
                    enabled: false,
                }
            ]
        );
        let json = serde_json::to_value(&document).expect("encode");
        assert_eq!(json["scope"], "owned");
        assert_eq!(json["authority"], "discovery");
        assert!(json["principals"][0].get("groups").is_none());
        assert!(json["principals"][0].get("grants").is_none());
    }

    #[test]
    fn invalid_or_duplicate_rows_fail_closed() {
        assert!(document_from_runtime_json(br#"[{"principal":"not/a/principal"}]"#).is_err());
        assert!(
            document_from_runtime_json(br#"[{"principal":"alice"},{"principal":"alice"}]"#)
                .is_err()
        );
        assert!(document_from_runtime_json(br#"{"principal":"alice"}"#).is_err());
        assert!(document_from_runtime_json(b"").is_err());
    }

    #[test]
    fn unknown_mine_flag_is_unsupported_and_auth_errors_are_not() {
        let unsupported = std::process::Command::new("sh")
            .args([
                "-c",
                "printf \"%s\" \"error: unexpected argument '--mine' found\" >&2; exit 2",
            ])
            .output()
            .expect("run unsupported fixture");
        assert!(runtime_discovery_unsupported(&unsupported));

        let auth = std::process::Command::new("sh")
            .args([
                "-c",
                "printf \"%s\" \"principal discovery requires a user-delegated device\" >&2; exit 1",
            ])
            .output()
            .expect("run auth fixture");
        assert!(!runtime_discovery_unsupported(&auth));

        let success = std::process::Command::new("sh")
            .args(["-c", "printf '[]'; exit 0"])
            .output()
            .expect("run success fixture");
        assert!(!runtime_discovery_unsupported(&success));
    }
}
