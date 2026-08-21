use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TEST_SECRET_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const TEST_CHANNEL: &str = "11111111-1111-4111-8111-111111111111";

fn base_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
    command
        .env("BUZZ_PRIVATE_KEY", TEST_SECRET_KEY)
        .env_remove("BUZZ_AUTH_TAG")
        .arg("--relay")
        .arg("http://127.0.0.1:9")
        .arg("messages")
        .arg("send")
        .arg("--channel")
        .arg(TEST_CHANNEL);
    command
}

#[test]
fn empty_literal_fails_before_contacting_relay() {
    for content in ["", " \n\t"] {
        let output = base_command()
            .arg("--content")
            .arg(content)
            .output()
            .expect("run buzz");
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("message must have content or attachments"));
        assert!(!stderr.contains("network_error"));
    }
}

#[test]
fn empty_stdin_at_eof_fails_before_contacting_relay() {
    let output = base_command()
        .arg("--content")
        .arg("-")
        .stdin(Stdio::null())
        .output()
        .expect("run buzz");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("message must have content or attachments"));
}

#[test]
fn open_non_tty_stdin_without_data_times_out() {
    let started = Instant::now();
    let mut child = base_command()
        .arg("--content")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn buzz");
    let writer = child.stdin.take().expect("piped stdin");

    let deadline = started + Duration::from_secs(7);
    loop {
        if child.try_wait().expect("poll child").is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed-out child");
            panic!("buzz remained blocked on an open, empty stdin pipe");
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    drop(writer);
    let output = child.wait_with_output().expect("collect output");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(started.elapsed() < Duration::from_secs(7));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("stdin produced no data within 5 seconds"));
    assert!(!stderr.contains("network_error"));
}

#[test]
fn slow_producer_that_eventually_writes_is_not_timed_out() {
    let started = Instant::now();
    let mut child = base_command()
        .arg("--content")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn buzz");
    let mut writer = child.stdin.take().expect("piped stdin");
    let producer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        writer.write_all(b"hello after a slow start\n")
    });

    let output = child.wait_with_output().expect("collect output");
    producer
        .join()
        .expect("join producer")
        .expect("write stdin");
    assert!(started.elapsed() >= Duration::from_secs(3));
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("network_error"), "{stderr}");
    assert!(!stderr.contains("stdin produced no data"));
}

#[test]
fn nonempty_stdin_reaches_the_normal_send_path() {
    let mut child = base_command()
        .arg("--content")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn buzz");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(b"hello from stdin")
        .expect("write stdin");
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("collect output");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("network_error"), "{stderr}");
    assert!(!stderr.contains("message must have content or attachments"));
}
