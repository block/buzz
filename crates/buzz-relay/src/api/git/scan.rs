//! The secret scan at the door: every object a push brings, on every ref,
//! read before the land and given to the gate as its `scan` input.
//!
//! ```text
//!   rev-list --objects <new tips> ^<tips in force> ─► cat-file --batch ─► SECRETS
//! ```
//!
//! What a push brings is everything its new tips reach that the tips it
//! replaces do not: the model's `Incoming`, not only the tips and not only
//! the governed refs. Anything short of a complete read is `Unknown`, which
//! the gate refuses like a detected secret.
//!
//! The scan reads the push's ephemeral workspace after receive-pack. Nothing
//! there is visible to anyone: the packs prepared from it are inert until
//! the land commits, and the land commits only a clean scan.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use buzz_git_gate::Scan;
use regex::bytes::Regex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

/// The scan is bounded like the other push-path Git work.
const SCAN_TIMEOUT: Duration = Duration::from_secs(300);

/// Objects are read in chunks; a match may straddle two, so each chunk is
/// scanned with the tail of the one before it. No pattern is longer.
const CHUNK: usize = 1 << 20;
const OVERLAP: usize = 256;

/// Credentials that are never meant to be in a repository: private keys,
/// Nostr secret keys, and cloud and forge access tokens.
static SECRETS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        // ASCII word boundaries: blobs are bytes, not text.
        r"(?-u)-----BEGIN [A-Z0-9 ]*PRIVATE KEY",
        r"|\bnsec1[02-9ac-hj-np-z]{58}\b",
        r"|\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        r"|\bgh[pousr]_[A-Za-z0-9]{36}\b",
        r"|\bgithub_pat_[A-Za-z0-9_]{82}\b",
        r"|\bxox[abprs]-[A-Za-z0-9-]{10,}",
    ))
    .expect("secret patterns compile")
});

/// Scan every object the push brings: reachable from `after`'s tips and not
/// from `before`'s. Runs in `repo_path`, which holds both.
pub(super) async fn push(
    repo_path: &Path,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Scan {
    match tokio::time::timeout(SCAN_TIMEOUT, objects(repo_path, before, after)).await {
        Ok(Ok(false)) => Scan::Clean,
        Ok(Ok(true)) => Scan::Dirty,
        Ok(Err(error)) => {
            tracing::warn!(%error, "git secret scan failed; the push is refused");
            Scan::Unknown
        }
        Err(_) => {
            tracing::warn!("git secret scan timed out; the push is refused");
            Scan::Unknown
        }
    }
}

/// Whether any object the push brings matches [`SECRETS`].
async fn objects(
    repo_path: &Path,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> std::io::Result<bool> {
    let mut revs = String::new();
    for oid in after.values().filter(|o| *o != ZERO_OID) {
        revs.push_str(oid);
        revs.push('\n');
    }
    if revs.is_empty() {
        return Ok(false);
    }
    for oid in before.values().filter(|o| *o != ZERO_OID) {
        revs.push('^');
        revs.push_str(oid);
        revs.push('\n');
    }

    let mut list = git(
        repo_path,
        &["rev-list", "--objects", "--no-object-names", "--stdin"],
    );
    list.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut list = list.spawn()?;
    let mut stdin = list.stdin.take().expect("piped");
    let oids: Stdio = list.stdout.take().expect("piped").try_into()?;
    let mut read = git(repo_path, &["cat-file", "--batch"]);
    read.stdin(oids).stdout(Stdio::piped());
    let mut read = read.spawn()?;
    let feed = async move {
        tokio::io::AsyncWriteExt::write_all(&mut stdin, revs.as_bytes()).await
        // Dropping stdin ends the list.
    };
    let mut out = BufReader::new(read.stdout.take().expect("piped"));
    let (fed, found) = tokio::join!(feed, scan_batch(&mut out));
    fed?;
    let found = found?;
    // A match stops reading early; anything after it no longer matters.
    if found {
        return Ok(true);
    }
    let (listed, read) = tokio::join!(list.wait(), read.wait());
    if !listed?.success() || !read?.success() {
        return Err(std::io::Error::other("git exited with an error"));
    }
    Ok(false)
}

/// Read `cat-file --batch` output to its end, or to the first match.
async fn scan_batch(out: &mut (impl AsyncBufReadExt + Unpin)) -> std::io::Result<bool> {
    let mut header = String::new();
    let mut buf = vec![0u8; OVERLAP + CHUNK];
    loop {
        header.clear();
        if out.read_line(&mut header).await? == 0 {
            return Ok(false);
        }
        // `<oid> <type> <size>`; anything else (`<oid> missing`) means the
        // push names an object it did not bring.
        let size: u64 = header
            .trim_end()
            .rsplit(' ')
            .next()
            .and_then(|s| s.parse().ok())
            .filter(|_| header.split(' ').count() == 3)
            .ok_or_else(|| std::io::Error::other(format!("unexpected batch header {header:?}")))?;
        let (mut left, mut kept) = (size, 0usize);
        while left > 0 {
            let want = CHUNK.min(usize::try_from(left).unwrap_or(CHUNK));
            out.read_exact(&mut buf[kept..kept + want]).await?;
            left -= want as u64;
            if SECRETS.is_match(&buf[..kept + want]) {
                return Ok(true);
            }
            let end = kept + want;
            let tail = end.min(OVERLAP);
            buf.copy_within(end - tail..end, 0);
            kept = tail;
        }
        let mut newline = [0u8; 1];
        out.read_exact(&mut newline).await?;
    }
}

fn git(repo_path: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(repo_path)
        .stderr(Stdio::null())
        .kill_on_drop(true);
    super::transport::harden_git_env(&mut cmd);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Built at run time so no credential-shaped literal is in the source.
    fn secrets() -> Vec<String> {
        vec![
            ["-----BEGIN ", "RSA PRIVATE", " KEY-----"].concat(),
            ["-----BEGIN ", "OPENSSH PRIVATE", " KEY-----"].concat(),
            ["nsec", "1", &"q".repeat(58)].concat(),
            ["AK", "IA", &"Z".repeat(16)].concat(),
            ["gh", "p_", &"a".repeat(36)].concat(),
            ["github", "_pat_", &"a".repeat(82)].concat(),
            ["xo", "xb-", &"1".repeat(12)].concat(),
        ]
    }

    #[test]
    fn the_patterns_find_each_secret_and_pass_near_misses() {
        for s in secrets() {
            assert!(SECRETS.is_match(format!("x = \"{s}\"\n").as_bytes()), "{s}");
        }
        for near in [
            "-----BEGIN PUBLIC KEY-----".to_string(),
            ["nsec", "1", &"q".repeat(57)].concat(),
            ["npub", "1", &"q".repeat(58)].concat(),
            ["AK", "IA", &"Z".repeat(15)].concat(),
            ["gh", "p_", &"a".repeat(35)].concat(),
        ] {
            assert!(!SECRETS.is_match(near.as_bytes()), "{near}");
        }
    }

    /// A batch stream of one object of `body`, preceded by `pad` bytes of
    /// another, so the match can be placed across a chunk boundary.
    fn batch(objects: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for body in objects {
            out.extend(format!("{} blob {}\n", "a".repeat(40), body.len()).as_bytes());
            out.extend(*body);
            out.push(b'\n');
        }
        out
    }

    async fn scan(stream: Vec<u8>) -> std::io::Result<bool> {
        scan_batch(&mut BufReader::new(std::io::Cursor::new(stream))).await
    }

    #[tokio::test]
    async fn a_match_is_found_across_a_chunk_boundary_but_not_across_objects() {
        let secret = &secrets()[2];
        let half = secret.len() / 2;
        for at in [0, CHUNK - half, CHUNK, 3 * CHUNK - 1] {
            let mut body = vec![b' '; at];
            body.extend(secret.as_bytes());
            body.extend(b" end");
            assert!(scan(batch(&[&body])).await.unwrap(), "secret at {at}");
        }
        let (a, b) = secret.as_bytes().split_at(half);
        assert!(
            !scan(batch(&[a, b])).await.unwrap(),
            "two objects never join"
        );
        assert!(!scan(batch(&[b"clean", &vec![0u8; CHUNK + 7]]))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn a_short_or_unexpected_stream_is_an_error() {
        let mut short = batch(&[b"0123456789"]);
        short.truncate(short.len() - 4);
        assert!(scan(short).await.is_err());
        let missing = format!("{} missing\n", "a".repeat(40)).into_bytes();
        assert!(scan(missing).await.is_err());
    }
}
