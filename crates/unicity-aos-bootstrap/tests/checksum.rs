use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn checksum_works_without_product_home_and_never_starts_runtime() {
    let root = std::env::temp_dir().join(format!(
        "aos-checksum-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let path = root.join("input with spaces");
    fs::write(&path, b"abc").unwrap();
    let home = root.join("absent-home");
    let output = Command::new(env!("CARGO_BIN_EXE_aos"))
        .args(["checksum", "--"])
        .arg(&path)
        .env("AOS_HOME", &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
    );
    assert!(!home.exists());
    let failure = Command::new(env!("CARGO_BIN_EXE_aos"))
        .arg("checksum")
        .arg(root.join("missing"))
        .env("AOS_HOME", &home)
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(failure.stdout.is_empty());
    assert!(!home.exists());
    #[cfg(unix)]
    {
        let fifo = root.join("fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_aos"))
            .arg("checksum")
            .arg(&fifo)
            .env("AOS_HOME", &home)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("checksum blocked on a FIFO");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!home.exists());
    }
    fs::remove_dir_all(root).unwrap();
}
