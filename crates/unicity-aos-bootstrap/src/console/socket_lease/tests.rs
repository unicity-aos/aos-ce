use super::*;
use crate::console::transport::Listener;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        // macOS's per-user temp directory alone can exceed the Unix socket limit.
        let path = Path::new("/tmp").join(format!("aos-console-lease-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).expect("owned test home");
        Self(path)
    }
}

#[test]
fn lease_drop_releases_lock_with_an_inherited_descriptor() {
    let home = Home::new();
    drop(Listener::bind(&home.0).expect("initial listener"));
    let directory = home.0.join("console");
    let lease = SocketLease::acquire(&directory).expect("lease");
    // dup shares the same open-file description as a descriptor inherited by fork.
    let inherited = lease._file.try_clone().expect("inherited description");
    drop(lease);
    assert!(SocketLease::acquire(&directory).is_ok());
    drop(inherited);
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn live_console_is_preserved_and_graceful_exit_allows_restart() {
    let home = Home::new();
    let first = Listener::bind(&home.0).expect("first listener");
    let socket = home.0.join("console/approval.sock");
    let inode = fs::symlink_metadata(&socket).expect("socket").ino();
    assert!(Listener::bind(&home.0).is_err());
    assert_eq!(
        fs::symlink_metadata(&socket).expect("preserved").ino(),
        inode
    );
    assert!(UnixStream::connect(&socket).is_ok());
    drop(first);
    assert!(!socket.exists());
    assert!(Listener::bind(&home.0).is_ok());
}

#[test]
fn crash_child() {
    let Some(home) = std::env::var_os("AOS_CONSOLE_CRASH_TEST_HOME") else {
        return;
    };
    let path = PathBuf::from(home);
    let stage = std::env::var("AOS_CONSOLE_CRASH_TEST_STAGE").unwrap_or_default();
    let _listener = Listener::bind_with_checkpoint(&path, |current| {
        if stage == format!("{current:?}") {
            fs::write(path.join("ready"), b"ready").expect("child readiness");
            loop {
                std::thread::park();
            }
        }
    })
    .expect("child listener");
    fs::write(path.join("ready"), b"ready").expect("child readiness");
    loop {
        std::thread::park();
    }
}

#[test]
fn killed_console_can_restart_without_manual_socket_removal() {
    for stage in ["Bound", "Configured", "Published", "Returned"] {
        kill_and_recover(stage);
    }
}

fn kill_and_recover(stage: &str) {
    let home = Home::new();
    let mut child = Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "console::socket_lease::tests::crash_child",
            "--nocapture",
        ])
        .env("AOS_CONSOLE_CRASH_TEST_HOME", &home.0)
        .env("AOS_CONSOLE_CRASH_TEST_STAGE", stage)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("owned crash child");
    let end = Instant::now() + Duration::from_secs(10);
    while !home.0.join("ready").exists() {
        if Instant::now() >= end || child.try_wait().expect("child state").is_some() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("crash child did not create listener");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().expect("kill only owned child");
    child.wait().expect("reap owned child");
    assert_eq!(
        home.0.join("console/approval.sock").exists(),
        matches!(stage, "Published" | "Returned")
    );
    let recovered = Listener::bind(&home.0).expect("recover crashed listener");
    assert!(UnixStream::connect(home.0.join("console/approval.sock")).is_ok());
    drop(recovered);
    assert!(!home.0.join("console/approval.sock").exists());
    assert!(!home.0.join("console/approval.owner.sock").exists());
}

#[test]
fn legacy_socket_without_record_is_preserved() {
    let home = Home::new();
    fs::create_dir(home.0.join("console")).expect("directory");
    fs::set_permissions(home.0.join("console"), fs::Permissions::from_mode(0o700)).expect("mode");
    let path = home.0.join("console/approval.sock");
    let legacy = UnixListener::bind(&path).expect("legacy");
    drop(legacy);
    let inode = fs::symlink_metadata(&path).expect("legacy socket").ino();
    assert!(Listener::bind(&home.0).is_err());
    assert_eq!(fs::symlink_metadata(&path).expect("preserved").ino(), inode);
}

#[test]
fn live_socket_is_preserved_even_with_a_matching_unheld_link() {
    let home = Home::new();
    drop(Listener::bind(&home.0).expect("initial listener"));
    let directory = home.0.join("console");
    let path = directory.join("approval.sock");
    let live = UnixListener::bind(&path).expect("live socket");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("private socket");
    let meta = fs::symlink_metadata(&path).expect("socket identity");
    fs::hard_link(&path, directory.join("approval.owner.sock")).expect("ownership link");
    assert!(Listener::bind(&home.0).is_err());
    assert_eq!(
        fs::symlink_metadata(&path).expect("preserved").ino(),
        meta.ino()
    );
    drop(live);
}

#[test]
fn obsolete_or_torn_lock_record_cannot_prevent_restart() {
    let home = Home::new();
    drop(Listener::bind(&home.0).expect("initial listener"));
    // Lock bytes are no longer authority; even an interrupted old writer is safe.
    for bytes in [b"".as_slice(), b"[123,", b"old record"] {
        fs::write(home.0.join("console/approval.lock"), bytes).expect("old record");
        drop(Listener::bind(&home.0).expect("restart ignores obsolete record"));
    }
}

#[test]
fn publication_never_overwrites_a_new_destination() {
    let home = Home::new();
    let path = home.0.join("console/approval.sock");
    let result = Listener::bind_with_checkpoint(&home.0, |stage| {
        if stage == BindStage::Configured {
            fs::write(&path, b"keep").expect("competing destination");
        }
    });
    assert!(result.is_err());
    assert_eq!(fs::read(path).expect("destination preserved"), b"keep");
}

#[test]
fn replaced_owner_is_not_reclaimed() {
    let home = Home::new();
    drop(Listener::bind(&home.0).expect("initial listener"));
    let owner = home.0.join("console/approval.owner.sock");
    fs::write(&owner, b"keep").expect("replacement owner");
    assert!(Listener::bind(&home.0).is_err());
    assert_eq!(fs::read(owner).expect("owner retained"), b"keep");
}

#[test]
fn replacement_file_and_symlink_are_not_removed() {
    for link in [false, true] {
        let home = Home::new();
        drop(Listener::bind(&home.0).expect("initial listener"));
        let socket = home.0.join("console/approval.sock");
        if link {
            symlink("missing", &socket).expect("replacement symlink");
        } else {
            fs::write(&socket, b"keep").expect("replacement file");
        }
        assert!(Listener::bind(&home.0).is_err());
        assert!(fs::symlink_metadata(&socket).is_ok());
    }
}

#[test]
fn symlinked_lock_is_rejected_without_changing_target() {
    let home = Home::new();
    drop(Listener::bind(&home.0).expect("initial listener"));
    let lock = home.0.join("console/approval.lock");
    fs::remove_file(&lock).expect("owned lock");
    let target = home.0.join("keep");
    fs::write(&target, b"untouched").expect("target");
    symlink(&target, &lock).expect("lock alias");
    assert!(Listener::bind(&home.0).is_err());
    assert_eq!(fs::read(target).expect("target retained"), b"untouched");
}
