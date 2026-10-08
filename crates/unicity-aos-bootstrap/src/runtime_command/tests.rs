use super::AosHome;

#[test]
fn product_daemon_selection_preserves_relative_arguments_and_caller_directory() {
    let home = AosHome::from_root("/tmp/aos-workspace-test");
    let command = home
        .runtime_command_with_args(["capsule", "build", "./local capsule", "--output", "./dist"])
        .expect("runtime command");
    let args = command.get_args().collect::<Vec<_>>();
    assert_eq!(
        args,
        [
            "--daemon-workspace",
            "/tmp/aos-workspace-test/runtime",
            "capsule",
            "build",
            "./local capsule",
            "--output",
            "./dist",
        ]
    );
    assert_eq!(command.get_current_dir(), None);
}
