//! Process-tree regression tests that launch subprocesses.

use std::os::unix::process::CommandExt;
use std::process::Command;

/// Regression coverage for managed-agent restart teardown. The helper test
/// process acts as `buzz-acp` and starts a child in its own process group,
/// mirroring `AcpClient::spawn`. Terminating the helper must reap both groups.
#[test]
fn terminate_process_reaps_independent_descendant_group() {
    let _path_guard = crate::managed_agents::lock_path_mutex();
    let marker_path = std::env::temp_dir().join(format!(
        "buzz-process-tree-{}-{}.pid",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&marker_path);

    let mut helper = {
        let mut command = Command::new(std::env::current_exe().expect("resolve test executable"));
        command
            .args([
                "--exact",
                "managed_agents::runtime::process_tree_tests::process_tree_descendant_helper",
                "--nocapture",
            ])
            .env("BUZZ_PROCESS_TREE_TEST_MARKER", &marker_path)
            .process_group(0);
        command.spawn().expect("spawn process-tree helper")
    };
    let helper_pid = helper.id();

    let child_pid = (0..100)
        .find_map(|_| {
            let value = std::fs::read_to_string(&marker_path).ok();
            if value.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            value.and_then(|pid| pid.trim().parse::<u32>().ok())
        })
        .expect("helper should report its independent child PID");

    assert_eq!(
        unsafe { libc::getpgid(child_pid as i32) },
        child_pid as i32,
        "helper child should lead an independent process group"
    );

    super::terminate_process(helper_pid).expect("terminate complete helper process tree");
    let _ = helper.wait();

    for _ in 0..50 {
        if !super::process_is_running(child_pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        !super::process_is_running(child_pid),
        "independently-grouped ACP child must not survive harness teardown"
    );
    let _ = std::fs::remove_file(marker_path);
}

/// Subprocess-only half of [`terminate_process_reaps_independent_descendant_group`].
#[test]
fn process_tree_descendant_helper() {
    let Some(marker_path) = std::env::var_os("BUZZ_PROCESS_TREE_TEST_MARKER") else {
        return;
    };
    let mut child = {
        let mut command = Command::new("sh");
        command
            .args(["-c", "while :; do sleep 60; done"])
            .process_group(0);
        command.spawn().expect("spawn independent helper child")
    };
    std::fs::write(marker_path, child.id().to_string()).expect("write helper child PID");
    let _ = child.wait();
}
