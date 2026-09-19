use std::ffi::OsString;

pub(super) fn runtime_start_requested(args: &[OsString]) -> bool {
    match super::leading_runtime_root_index(args) {
        Ok(Some(index)) => args
            .get(index)
            .is_some_and(|root| root == "start" || root == "restart"),
        Ok(None) => false,
        Err(()) => super::fallback_runtime_root(args)
            .is_some_and(|root| root == "start" || root == "restart"),
    }
}

#[cfg(target_os = "macos")]
pub(super) fn open_command_center() {
    use std::process::{Command, Stdio};

    let Some(home) = std::env::var_os("HOME") else {
        eprintln!("aos: HOME is unset; AOS Command Center was not opened");
        return;
    };
    let home = std::path::PathBuf::from(home);
    if !home.is_absolute() {
        eprintln!("aos: HOME is not absolute; AOS Command Center was not opened");
        return;
    }

    let app = home.join("Applications/AOS Command Center.app");
    let Ok(metadata) = std::fs::symlink_metadata(&app) else {
        return;
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        eprintln!(
            "aos: refusing to open invalid AOS Command Center path {}",
            app.display()
        );
        return;
    }

    match Command::new("/usr/bin/open")
        .arg("-g")
        .arg(&app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            // Neither LaunchServices output nor inherited pipe ownership may
            // interfere with MCP's stdio connection. Reap without blocking it.
            std::thread::spawn(move || match child.wait() {
                Ok(status) if status.success() => {}
                _ => eprintln!(
                    "aos: AOS Command Center could not be opened. Open the installed app manually to review requests."
                ),
            });
        }
        Err(error) => eprintln!("aos: failed to open AOS Command Center: {error}"),
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn open_command_center() {}

#[cfg(test)]
mod tests {
    use super::runtime_start_requested;
    use std::ffi::OsString;

    #[test]
    fn opens_only_for_daemon_start_roots() {
        assert!(runtime_start_requested(&[OsString::from("start")]));
        assert!(runtime_start_requested(&[OsString::from("restart")]));
        assert!(runtime_start_requested(&[
            OsString::from("--principal"),
            OsString::from("operator"),
            OsString::from("start"),
        ]));
        assert!(!runtime_start_requested(&[OsString::from("status")]));
        assert!(!runtime_start_requested(&[
            OsString::from("capsule"),
            OsString::from("start"),
        ]));
    }
}
