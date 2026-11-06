//! Fetch the agent's signing key from the local key broker.
//!
//! A process's launch environment can be read by any process under the same
//! OS user, and children inherit it. With `BUZZ_KEY_BROKER_ACCOUNT` set, the
//! key never enters this process's environment: it is read once from the
//! broker's owner-only UNIX socket into memory, and children get only the
//! account name.
//!
//! Protocol: send `GET <account>\n`, receive `OK <base64>\n` or `ERR <reason>\n`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use base64::Engine;
use zeroize::Zeroizing;

use crate::config::ConfigError;

pub(crate) const ACCOUNT_ENV: &str = "BUZZ_KEY_BROKER_ACCOUNT";
const SOCKET_ENV: &str = "BUZZ_KEY_BROKER_SOCKET";
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const ATTEMPTS: u32 = 5;
const RETRY_DELAY: Duration = Duration::from_secs(2);

fn socket_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = std::env::var_os(SOCKET_ENV) {
        return Ok(PathBuf::from(path));
    }
    let home =
        std::env::var_os("HOME").ok_or_else(|| ConfigError::KeyBroker("HOME is not set".into()))?;
    Ok(Path::new(&home).join(".buzz/broker.sock"))
}

fn request(socket: &Path, account: &str) -> std::io::Result<Zeroizing<String>> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    writeln!(stream, "GET {account}")?;
    let mut line = Zeroizing::new(String::new());
    BufReader::new(stream).read_line(&mut line)?;
    Ok(line)
}

/// Read one account's key. The broker can start after this process at boot,
/// so a socket failure is retried briefly. A refusal from the broker is final.
pub(crate) fn fetch(account: &str) -> Result<Zeroizing<String>, ConfigError> {
    let socket = socket_path()?;
    let mut last_error = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            sleep(RETRY_DELAY);
        }
        match request(&socket, account) {
            Ok(line) => return parse_reply(&line),
            Err(error) => last_error = Some(error),
        }
    }
    Err(ConfigError::KeyBroker(format!(
        "could not reach {}: {}",
        socket.display(),
        last_error.map_or_else(|| "no attempt ran".to_string(), |e| e.to_string())
    )))
}

/// Git runs the credential and signing helpers as short-lived children of the
/// agent. In broker mode they get the key here, for their own process only.
/// A failure is reported, and the helper then fails with its own missing-key error.
pub(crate) fn prime_git_helper() {
    let Ok(account) = std::env::var(ACCOUNT_ENV) else {
        return;
    };
    match fetch(&account) {
        // Called from the helper entry point, before any thread starts.
        Ok(key) => std::env::set_var("NOSTR_PRIVATE_KEY", key.as_str()),
        Err(error) => eprintln!("buzz-acp git helper: {error}"),
    }
}

fn parse_reply(line: &str) -> Result<Zeroizing<String>, ConfigError> {
    let line = line.trim();
    let Some(encoded) = line.strip_prefix("OK ") else {
        // Only a broker's own short reason is ever echoed, never a key reply.
        let reason = line.strip_prefix("ERR ").unwrap_or("malformed reply");
        return Err(ConfigError::KeyBroker(format!("broker refused: {reason}")));
    };
    let bytes = Zeroizing::new(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| ConfigError::KeyBroker("broker reply is not base64".into()))?,
    );
    let key = std::str::from_utf8(&bytes)
        .map_err(|_| ConfigError::KeyBroker("broker reply is not UTF-8".into()))?;
    Ok(Zeroizing::new(key.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn serve_once(
        reply: &'static str,
    ) -> (tempfile::TempDir, PathBuf, std::thread::JoinHandle<String>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broker.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            stream.write_all(reply.as_bytes()).unwrap();
            line
        });
        (dir, path, handle)
    }

    #[test]
    fn asks_for_the_account_and_decodes_the_value() {
        let (_dir, path, handle) = serve_once("OK dGVzdC12YWx1ZQ==\n");
        let line = request(&path, "claude-agent-private-key").unwrap();
        assert_eq!(parse_reply(&line).unwrap().as_str(), "test-value");
        assert_eq!(handle.join().unwrap(), "GET claude-agent-private-key\n");
    }

    #[test]
    fn a_refusal_names_the_reason() {
        let error = parse_reply("ERR not allowlisted\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("not allowlisted"), "{error}");
    }

    #[test]
    fn a_malformed_reply_never_echoes_its_content() {
        let error = parse_reply("not-a-reply-with-secret-looking-text\n")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret-looking"), "{error}");
        let error = parse_reply("OK !!!not-base64!!!\n")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("not-base64!!!"), "{error}");
    }
}
