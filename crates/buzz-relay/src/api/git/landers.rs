//! Who landed what: the commits a push brings, read before the land, so the
//! land can record their first lander and find the authors among the
//! approvers (`docs/spec/GitGovernance.tla`, `pushed` and `Authors`).
//!
//! ```text
//!   rev-list <new tips> ^<tips in force>     ─► incoming: recorded after the swap
//!   rev-list --max-count=N+1 <new> ^<old>    ─► each ref's brings: its authors
//! ```
//!
//! Incoming is the scan's set, commits only. A ref's brings are bounded: past
//! [`MAX_BRINGS`] the land cannot tell who did the work, so it refuses the ref
//! if an approval of it could count, and lands it otherwise.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use buzz_core::git_perms::RefUpdate;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

/// Commits a ref change may bring and still have its authors found.
pub(super) const MAX_BRINGS: usize = 10_000;

/// Bounded like the scan.
const TIMEOUT: Duration = Duration::from_secs(300);

/// What a push brings, as the land needs it.
#[derive(Debug, Default)]
pub(super) struct Brought {
    /// Commits the new tips reach that no tip in force does.
    pub incoming: Vec<String>,
    /// Per update, in order: the commits its new tip reaches that its old
    /// one does not, or `None` past [`MAX_BRINGS`].
    pub brings: Vec<Option<Vec<String>>>,
}

/// Read what the push from `before` to `after` brings. Runs in `repo_path`,
/// which holds both. An error means the land cannot know, and must not run.
pub(super) async fn read(
    repo_path: &Path,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    updates: &[RefUpdate],
) -> std::io::Result<Brought> {
    let read = async {
        let mut revs = String::new();
        for oid in after.values().filter(|o| *o != ZERO_OID) {
            revs.push_str(&format!("{oid}\n"));
        }
        for oid in before.values().filter(|o| *o != ZERO_OID) {
            revs.push_str(&format!("^{oid}\n"));
        }
        let incoming = if after.values().any(|o| o != ZERO_OID) {
            rev_list(repo_path, &[], &revs).await?
        } else {
            Vec::new()
        };
        let mut brings = Vec::with_capacity(updates.len());
        for u in updates {
            if u.new_oid == ZERO_OID {
                brings.push(Some(Vec::new()));
                continue;
            }
            let mut revs = format!("{}\n", u.new_oid);
            if u.old_oid != ZERO_OID {
                revs.push_str(&format!("^{}\n", u.old_oid));
            }
            let max = format!("--max-count={}", MAX_BRINGS + 1);
            let b = rev_list(repo_path, &[&max], &revs).await?;
            brings.push((b.len() <= MAX_BRINGS).then_some(b));
        }
        Ok(Brought { incoming, brings })
    };
    tokio::time::timeout(TIMEOUT, read)
        .await
        .map_err(|_| std::io::Error::other("timed out"))?
}

/// `git rev-list <args> --stdin`, fed `revs`; the commit ids it lists.
async fn rev_list(repo_path: &Path, args: &[&str], revs: &str) -> std::io::Result<Vec<String>> {
    let mut cmd = Command::new("git");
    cmd.arg("rev-list")
        .args(args)
        .arg("--stdin")
        .current_dir(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    super::transport::harden_git_env(&mut cmd);
    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take().expect("piped");
    let feed = async move { stdin.write_all(revs.as_bytes()).await };
    let (fed, out) = tokio::join!(feed, child.wait_with_output());
    fed?;
    let out = out?;
    if !out.status.success() {
        return Err(std::io::Error::other("git rev-list failed"));
    }
    let listed = String::from_utf8(out.stdout).map_err(std::io::Error::other)?;
    Ok(listed.lines().map(str::to_string).collect())
}
