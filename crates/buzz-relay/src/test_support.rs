const DEFAULT_DATABASE_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

/// Resolve the database URL shared by PostgreSQL-backed relay tests.
pub(crate) fn database_url() -> String {
    std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("TEST_DATABASE_URL"))
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_owned())
}

#[cfg(test)]
const CHILD_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(test)]
const MAX_CAPTURE_BYTES: u64 = 1024 * 1024;

#[cfg(test)]
struct CapturedStream {
    retained: Vec<u8>,
    total_bytes: u64,
}

#[cfg(test)]
fn capture_stream(mut stream: impl std::io::Read) -> CapturedStream {
    let mut retained = Vec::new();
    let mut total_bytes = 0_u64;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = stream.read(&mut chunk).expect("read child output pipe");
        if read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(u64::try_from(read).expect("read size fits u64"));
        let remaining = usize::try_from(MAX_CAPTURE_BYTES)
            .expect("capture ceiling fits usize")
            .saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..read.min(remaining)]);
    }
    CapturedStream {
        retained,
        total_bytes,
    }
}

#[cfg(test)]
fn join_capture(capture: std::thread::JoinHandle<CapturedStream>, stream: &str) -> Vec<u8> {
    let capture = capture.join().expect("child capture thread must not panic");
    assert!(
        capture.total_bytes <= MAX_CAPTURE_BYTES,
        "child {stream} exceeded {MAX_CAPTURE_BYTES} bytes: {}",
        capture.total_bytes,
    );
    capture.retained
}

/// Run exactly one unit test in an isolated, deadline-bounded child process.
#[cfg(test)]
pub(crate) fn run_exact_test_child(test_name: &str, child_env: &str) {
    use std::{
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    let mut child = Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env(child_env, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn isolated test child");
    let stdout = child.stdout.take().expect("child stdout pipe");
    let stderr = child.stderr.take().expect("child stderr pipe");
    let stdout = thread::spawn(move || capture_stream(stdout));
    let stderr = thread::spawn(move || capture_stream(stderr));

    let deadline = Instant::now() + CHILD_TEST_TIMEOUT;
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().expect("poll isolated test child") {
            break (status, false);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let status = child.wait().expect("reap timed-out test child");
            break (status, true);
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout = join_capture(stdout, "stdout");
    let stderr = join_capture(stderr, "stderr");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );

    assert!(
        !timed_out,
        "isolated test child exceeded {CHILD_TEST_TIMEOUT:?}:\n{output}"
    );
    assert!(status.success(), "isolated test child failed:\n{output}");
    assert!(
        output.contains("running 1 test") && output.contains(test_name),
        "exact selector did not run the intended test {test_name}:\n{output}"
    );
}
