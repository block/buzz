//! Shared, read-only canonical Git observations for BW producers and consumers.
//!
//! The clone URL comes only from the repository genesis that Core authenticated,
//! and branch values are validated before they can reach a `git` argv.  Callers
//! still pass every observation back through Core; this module never decides a
//! BW transition by itself.

use super::project_git_exec::{run_git, GitAuthConfig};
use crate::bw_projection;
use buzz_core_pkg::bw::parse_json;

/// Defensive shape gate before a stream name reaches a `git` argv, mirroring
/// (not replacing) NIP-BW.md's own `stream` grammar.
pub(crate) fn validate_stream_for_git(stream: &str) -> Result<(), String> {
    let bytes = stream.as_bytes();
    let ok = !stream.is_empty()
        && stream.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'/' | b'-')
        })
        && !stream.contains("..")
        && !stream.contains("@{")
        && !stream.ends_with(['.', '/'])
        && stream
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"));
    if ok {
        Ok(())
    } else {
        Err("Invalid BW stream name.".to_string())
    }
}

/// Return the clone URL from the exact repository genesis authenticated by
/// Core's `activation()`, never from another fetched announcement.
pub(crate) fn genesis_clone_url(
    bw_input: &bw_projection::Input,
    genesis_id: &str,
) -> Result<String, String> {
    bw_input
        .events
        .iter()
        .find_map(|raw| {
            let wire = parse_json(raw.as_bytes()).ok()?;
            if wire["id"].as_str() != Some(genesis_id) {
                return None;
            }
            wire["tags"].as_array()?.iter().find_map(|tag| {
                let tag = tag.as_array()?;
                (tag.first()?.as_str()? == "clone")
                    .then(|| tag.get(1)?.as_str())
                    .flatten()
                    .map(str::to_owned)
            })
        })
        .ok_or_else(|| "Repository announcement has no clone URL on record.".to_string())
}

/// Read the exact current `refs/heads/<stream>` commit from canonical Git.
/// Tail-matching shadow refs returned by `git ls-remote` are ignored.
pub(crate) fn resolve_stream_head_blocking(
    clone_url: &str,
    stream: &str,
    auth: &GitAuthConfig,
) -> Result<String, String> {
    let refname = format!("refs/heads/{stream}");
    let output = run_git(
        &[
            "ls-remote",
            "--exit-code",
            "--end-of-options",
            clone_url,
            refname.as_str(),
        ],
        None,
        auth,
    )
    .map_err(|error| format!("Could not read the repository's current {stream} head: {error}"))?;
    // `ls-remote` patterns also match after a `/` boundary. Only the exact
    // branch ref is authoritative; a writer may be able to push shadow refs.
    output
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let sha = parts.next()?;
            let matched_ref = parts.next()?;
            (matched_ref == refname).then(|| sha.to_ascii_lowercase())
        })
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| format!("Could not resolve a commit for stream {stream}."))
}
