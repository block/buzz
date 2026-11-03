use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::process::Output;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use buzz_auth::nip_fi::{HttpJwksFetcher, JwksFetcher, JwksSourceContract};

const CHILD_MARKER: &str = "BUZZ_AUTH_JWKS_PROXY_TEST_CHILD";
const CHILD_TARGET: &str = "BUZZ_AUTH_JWKS_PROXY_TEST_TARGET";
const JWKS_BODY: &str = r#"{"keys":[{"kty":"EC","crv":"P-256","x":"f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU","y":"x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0","use":"sig","alg":"ES256","kid":"proxy-test"}]}"#;
const SHUTDOWN_PATH: &str = "/__mesh_jwks_proxy_test_shutdown__";

#[test]
fn mesh_fetch_for_contract_ignores_proxy_environment() {
    if std::env::var_os(CHILD_MARKER).is_some() {
        run_child_test();
    } else {
        run_parent_test();
    }
}

fn run_parent_test() {
    let mut servers = LocalServers::start().expect("start local JWKS and proxy listeners");
    let proxy_url = format!("http://{}", servers.proxy_addr);
    let target_url = format!("http://localhost:{}", servers.target_addr.port());
    let executable = std::env::current_exe().expect("current integration-test binary");

    let mut command = tokio::process::Command::new(executable);
    command
        .args([
            "--exact",
            "mesh_fetch_for_contract_ignores_proxy_environment",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env(CHILD_TARGET, target_url)
        .env("HTTP_PROXY", &proxy_url)
        .env("http_proxy", &proxy_url)
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("REQUEST_METHOD")
        .kill_on_drop(true);

    // The child receives a controlled proxy environment at process creation,
    // before it creates its current-thread Tokio runtime. Its test process
    // cannot alter or race the environment of the other buzz-auth tests.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build parent runtime");
    let output = runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(20), command.output()).await });

    let shutdown = servers.shutdown();
    shutdown.expect("stop local proxy and JWKS servers");
    let output = output.expect("proxy regression child must finish within 20 seconds");
    let output = output.expect("run proxy regression child");
    assert_child_succeeded(output);
}

fn run_child_test() {
    let target_url = std::env::var(CHILD_TARGET).expect("parent provides local JWKS URL");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build child current-thread runtime");

    runtime.block_on(async move {
        let default_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("build default-proxy positive control client");
        let control = tokio::time::timeout(
            Duration::from_secs(3),
            default_client
                .get(format!("{target_url}/proxy-control"))
                .send(),
        )
        .await
        .expect("default client should reach the local proxy")
        .expect("local proxy should return its positive-control response");
        assert_eq!(control.status(), reqwest::StatusCode::BAD_GATEWAY);
        assert_eq!(
            control
                .headers()
                .get("x-test-proxy")
                .and_then(|value| value.to_str().ok()),
            Some("intercepted"),
            "positive control proves proxy environment variables are active"
        );

        let contract = JwksSourceContract::new_mesh(format!("{target_url}/jwks.json"), 300, 3600)
            .expect("valid sealed mesh JWKS contract");
        let body = tokio::time::timeout(
            Duration::from_secs(12),
            HttpJwksFetcher::new().fetch_for_contract(&contract),
        )
        .await
        .expect("contract fetch is bounded")
        .expect("mesh fetch must bypass the configured proxy and reach loopback");
        assert_eq!(body, JWKS_BODY);
    });
}

fn assert_child_succeeded(output: Output) {
    assert!(
        output.status.success(),
        "proxy regression child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct LocalServers {
    target_addr: SocketAddr,
    proxy_addr: SocketAddr,
    target_thread: Option<JoinHandle<io::Result<()>>>,
    proxy_thread: Option<JoinHandle<io::Result<()>>>,
}

impl LocalServers {
    fn start() -> io::Result<Self> {
        let target_addr = ("localhost", 0)
            .to_socket_addrs()?
            .next()
            .filter(|address| address.ip().is_loopback())
            .ok_or_else(|| io::Error::other("localhost has no loopback address"))?;
        let target_listener = TcpListener::bind(target_addr)?;
        let proxy_listener = TcpListener::bind(("127.0.0.1", 0))?;
        let target_addr = target_listener.local_addr()?;
        let proxy_addr = proxy_listener.local_addr()?;
        let target_thread = thread::spawn(move || serve_target(target_listener));
        let proxy_thread = thread::spawn(move || serve_proxy(proxy_listener));

        Ok(Self {
            target_addr,
            proxy_addr,
            target_thread: Some(target_thread),
            proxy_thread: Some(proxy_thread),
        })
    }

    fn shutdown(&mut self) -> Result<(), String> {
        if self
            .target_thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            send_shutdown(self.target_addr);
        }
        if self
            .proxy_thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            send_shutdown(self.proxy_addr);
        }

        join_server(&mut self.target_thread)?;
        join_server(&mut self.proxy_thread)
    }
}

impl Drop for LocalServers {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn join_server(handle: &mut Option<JoinHandle<io::Result<()>>>) -> Result<(), String> {
    let Some(handle) = handle.take() else {
        return Ok(());
    };
    handle
        .join()
        .map_err(|_| "local server thread panicked".to_owned())?
        .map_err(|error| error.to_string())
}

fn send_shutdown(address: SocketAddr) {
    if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(1)) {
        let _ = stream.write_all(
            format!("GET {SHUTDOWN_PATH} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        );
    }
}

fn serve_proxy(listener: TcpListener) -> io::Result<()> {
    let (mut stream, _) = listener.accept()?;
    set_stream_timeouts(&stream)?;
    let request = read_request_headers(&mut stream)?;
    if request
        .lines()
        .next()
        .unwrap_or_default()
        .contains(SHUTDOWN_PATH)
    {
        return Ok(());
    }

    stream.write_all(
        b"HTTP/1.1 502 Bad Gateway\r\nX-Test-Proxy: intercepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
}

fn serve_target(listener: TcpListener) -> io::Result<()> {
    let (mut stream, _) = listener.accept()?;
    set_stream_timeouts(&stream)?;
    let request = read_request_headers(&mut stream)?;
    let first_line = request.lines().next().unwrap_or_default();
    if first_line.contains(SHUTDOWN_PATH) {
        return Ok(());
    }

    if first_line.starts_with("GET /jwks.json ") {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            JWKS_BODY.len(),
            JWKS_BODY
        );
        return stream.write_all(response.as_bytes());
    }

    stream.write_all(
        b"HTTP/1.1 418 Unexpected Target Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
}

fn set_stream_timeouts(stream: &TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))
}

fn read_request_headers(stream: &mut TcpStream) -> io::Result<String> {
    const MAX_HEADER_BYTES: usize = 16 * 1024;

    let mut request = Vec::new();
    let mut chunk = [0; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if request.len() > MAX_HEADER_BYTES {
            return Err(io::Error::other("request headers exceed test limit"));
        }
    }
    Ok(String::from_utf8_lossy(&request).into_owned())
}
