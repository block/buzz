//! One-way, exact-ref synchronization from GitHub `main` to Buzz `main`.
//!
//! These commands intentionally do not publish Nostr status events. The
//! caller-owned kind:30617 announcement is the only source of remote URLs,
//! while GitHub remains the final authority for the commit to copy.

use std::ffi::OsStr;
use std::process::{Command, Output};

use nostr::Event;
use tempfile::TempDir;

use crate::client::BuzzClient;
use crate::error::CliError;

const MAIN_REF: &str = "refs/heads/main";
const GITHUB_TRACKING_REF: &str = "refs/remotes/github/main";
const BUZZ_TRACKING_REF: &str = "refs/remotes/buzz/main";

#[derive(Debug, Clone)]
struct RepoRemotes {
    repo_id: String,
    buzz_url: String,
    github_url: String,
}

#[derive(Clone)]
struct GitAuth {
    private_key: String,
    auth_tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteState {
    main: Option<String>,
    head: Option<String>,
    head_target: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct StatusOutput<'a> {
    repo_id: &'a str,
    direction: &'static str,
    github_url: &'a str,
    buzz_url: &'a str,
    github_main: &'a str,
    buzz_main: Option<&'a str>,
    buzz_head: Option<&'a str>,
    in_sync: bool,
}

#[derive(Debug, serde::Serialize)]
struct WriteOutput {
    repo_id: String,
    direction: &'static str,
    commit: String,
    changed: bool,
    github_main: String,
    buzz_main: String,
    buzz_head: String,
}

fn exact_oid(value: &str, flag: &str) -> Result<String, CliError> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::Usage(format!(
            "{flag} must be an exact 40-hex commit"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

fn exact_tag_values<'a>(event: &'a Event, name: &str) -> Result<Option<Vec<&'a str>>, CliError> {
    let mut matching = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(name));
    let Some(tag) = matching.next() else {
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(CliError::Usage(format!(
            "repository announcement has multiple {name} tags"
        )));
    }
    Ok(Some(
        tag.as_slice().iter().skip(1).map(String::as_str).collect(),
    ))
}

pub(super) fn validate_github_clone(value: &str) -> Result<(), CliError> {
    let url = url::Url::parse(value)
        .map_err(|_| CliError::Usage("GitHub clone URL must be a valid HTTPS URL".into()))?;
    let path_is_repo = url
        .path_segments()
        .map(|parts| {
            let parts: Vec<&str> = parts.collect();
            parts.len() == 2 && parts.iter().all(|part| !part.is_empty())
        })
        .unwrap_or(false);
    if url.scheme() != "https"
        || url
            .host_str()
            .map(|host| host.eq_ignore_ascii_case("github.com"))
            != Some(true)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !path_is_repo
    {
        return Err(CliError::Usage(
            "GitHub clone URL must be https://github.com/<owner>/<repo>[.git] without credentials, query, or fragment"
                .into(),
        ));
    }
    Ok(())
}

fn derive_remotes(client: &BuzzClient, announcement: &Event) -> Result<RepoRemotes, CliError> {
    // A NIP-OA tag delegates authority to this signing identity; it does not
    // replace the event author or the repository path owner. Git repository
    // announcements and `/git/<owner>/<repo>` paths remain keyed by the
    // signing key that authored kind:30617.
    let owner = client.keys().public_key().to_hex();
    if announcement.pubkey.to_hex() != owner {
        return Err(CliError::Auth(
            "repository announcement is not owned by the effective caller".into(),
        ));
    }

    let repo_id = exact_tag_values(announcement, "d")?
        .and_then(|values| values.first().copied())
        .ok_or_else(|| CliError::Other("repository announcement is missing its d tag".into()))?
        .to_owned();
    crate::validate::validate_repo_id(&repo_id)?;
    let buzz_url = format!(
        "{}/git/{owner}/{repo_id}",
        client.relay_url().trim_end_matches('/')
    );
    let clones = exact_tag_values(announcement, "clone")?
        .filter(|values| !values.is_empty())
        .ok_or_else(|| CliError::Usage("repository announcement has no clone URLs".into()))?;
    if clones.first().copied() != Some(buzz_url.as_str()) {
        return Err(CliError::Usage(format!(
            "repository clone tag must list the canonical Buzz URL first: {buzz_url}"
        )));
    }

    let github: Vec<&str> = clones
        .iter()
        .skip(1)
        .copied()
        .filter(|value| {
            url::Url::parse(value).ok().and_then(|url| {
                url.host_str()
                    .map(|host| host.eq_ignore_ascii_case("github.com"))
            }) == Some(true)
        })
        .collect();
    if clones.len() != github.len() + 1 {
        return Err(CliError::Usage(
            "repository clone tag may contain only canonical Buzz followed by an optional GitHub clone"
                .into(),
        ));
    }
    let github_url = match github.as_slice() {
        [url] => (*url).to_owned(),
        [] => {
            return Err(CliError::Usage(
                "one GitHub clone URL is required for GitHub-to-Buzz sync".into(),
            ))
        }
        _ => {
            return Err(CliError::Usage(
                "repository announcement has multiple GitHub clone URLs".into(),
            ))
        }
    };
    validate_github_clone(&github_url)?;

    Ok(RepoRemotes {
        repo_id,
        buzz_url,
        github_url,
    })
}

struct GitRepo {
    _temp: TempDir,
    git_dir: std::path::PathBuf,
    auth: GitAuth,
}

impl GitRepo {
    fn new(auth: GitAuth) -> Result<Self, CliError> {
        let work_root = dirs::home_dir()
            .ok_or_else(|| CliError::Other("home directory is unavailable".into()))?
            .join("work");
        std::fs::create_dir_all(&work_root)
            .map_err(|_| CliError::Other("failed to prepare the private sync workspace".into()))?;
        let temp = tempfile::Builder::new()
            .prefix("buzz-repo-sync-")
            .tempdir_in(work_root)
            .map_err(|_| CliError::Other("failed to create private temporary repository".into()))?;
        let git_dir = temp.path().join("repo.git");
        let repo = Self {
            _temp: temp,
            git_dir,
            auth,
        };
        repo.run(false, "initialize temporary repository", ["init", "--bare"])?;
        Ok(repo)
    }

    fn command(&self, authenticated: bool) -> Command {
        let mut command = Command::new("git");
        let path = std::env::var_os("PATH").unwrap_or_default();
        command
            .env_clear()
            .env("PATH", path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.git_dir.join("no-global-config"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .arg("--git-dir")
            .arg(&self.git_dir)
            .arg("-c")
            .arg(format!(
                "core.hooksPath={}",
                self.git_dir.join("no-hooks").display()
            ))
            .args(["-c", "credential.helper="])
            .args(["-c", "credential.useHttpPath=true"]);
        if authenticated {
            command
                .args(["-c", "credential.helper=nostr"])
                .env("NOSTR_PRIVATE_KEY", &self.auth.private_key);
            if let Some(auth_tag) = &self.auth.auth_tag {
                command.env("BUZZ_AUTH_TAG", auth_tag);
            }
        }
        command
    }

    fn output<I, S>(
        &self,
        authenticated: bool,
        operation: &str,
        args: I,
    ) -> Result<Output, CliError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self
            .command(authenticated)
            .args(args)
            .output()
            .map_err(|_| CliError::Other(format!("failed to run git for {operation}")))?;
        if !output.status.success() {
            return Err(CliError::Other(format!(
                "git {operation} failed (exit {})",
                output.status.code().unwrap_or(-1)
            )));
        }
        Ok(output)
    }

    fn run<I, S>(&self, authenticated: bool, operation: &str, args: I) -> Result<(), CliError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.output(authenticated, operation, args).map(|_| ())
    }

    fn ls_remote(&self, url: &str, authenticated: bool) -> Result<RemoteState, CliError> {
        let output = self.output(
            authenticated,
            "read remote refs",
            ["ls-remote", "--symref", url, "HEAD", MAIN_REF],
        )?;
        let stdout = std::str::from_utf8(&output.stdout)
            .map_err(|_| CliError::Other("git returned invalid ref data".into()))?;
        parse_remote_state(stdout)
    }

    fn github_main(&self, url: &str) -> Result<String, CliError> {
        self.ls_remote(url, false)?
            .main
            .ok_or_else(|| CliError::NotFound("GitHub main is absent".into()))
    }

    fn fetch_github_main(&self, url: &str, commit: &str) -> Result<(), CliError> {
        let refspec = format!("+{MAIN_REF}:{GITHUB_TRACKING_REF}");
        self.run(
            false,
            "fetch GitHub main",
            ["fetch", "--no-tags", "--force", url, refspec.as_str()],
        )?;
        let fetched = self.rev_parse(GITHUB_TRACKING_REF)?;
        if fetched != commit {
            return Err(CliError::Conflict(
                "GitHub main changed while it was being fetched; retry with the new commit".into(),
            ));
        }
        Ok(())
    }

    fn fetch_buzz_main(&self, url: &str) -> Result<(), CliError> {
        let refspec = format!("+{MAIN_REF}:{BUZZ_TRACKING_REF}");
        self.run(
            true,
            "fetch Buzz main",
            ["fetch", "--no-tags", "--force", url, refspec.as_str()],
        )
    }

    fn rev_parse(&self, reference: &str) -> Result<String, CliError> {
        let output = self.output(
            false,
            "resolve commit",
            ["rev-parse", "--verify", reference],
        )?;
        let value = std::str::from_utf8(&output.stdout)
            .map_err(|_| CliError::Other("git returned invalid commit data".into()))?
            .trim();
        exact_oid(value, "resolved ref")
    }

    fn is_ancestor(&self, older: &str, newer: &str) -> Result<bool, CliError> {
        let output = self
            .command(false)
            .args(["merge-base", "--is-ancestor", older, newer])
            .output()
            .map_err(|_| CliError::Other("failed to run git for ancestry proof".into()))?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(CliError::Other("git ancestry proof failed".into())),
        }
    }

    fn push_main(&self, url: &str, commit: &str, expected: Option<&str>) -> Result<(), CliError> {
        let lease = format!(
            "--force-with-lease={MAIN_REF}:{}",
            expected.unwrap_or_default()
        );
        let refspec = format!("{commit}:{MAIN_REF}");
        self.run(
            true,
            "push Buzz main",
            [
                "push",
                "--porcelain",
                "--no-follow-tags",
                lease.as_str(),
                url,
                refspec.as_str(),
            ],
        )
    }
}

fn parse_remote_state(stdout: &str) -> Result<RemoteState, CliError> {
    let mut state = RemoteState {
        main: None,
        head: None,
        head_target: None,
    };
    for line in stdout.lines() {
        let Some((left, name)) = line.split_once('\t') else {
            return Err(CliError::Other("git returned malformed ref data".into()));
        };
        match name {
            "HEAD" if left.starts_with("ref: ") => {
                state.head_target = Some(left[5..].to_owned());
            }
            "HEAD" => state.head = Some(exact_oid(left, "remote HEAD")?),
            MAIN_REF => state.main = Some(exact_oid(left, "remote main")?),
            _ => {}
        }
    }
    Ok(state)
}

fn require_exact_head(state: &RemoteState, commit: &str) -> Result<(), CliError> {
    if state.main.as_deref() != Some(commit)
        || state.head.as_deref() != Some(commit)
        || state.head_target.as_deref() != Some(MAIN_REF)
    {
        return Err(CliError::Conflict(
            "Buzz main and HEAD did not read back at the exact requested commit".into(),
        ));
    }
    Ok(())
}

fn auth_from_client(client: &BuzzClient) -> GitAuth {
    GitAuth {
        private_key: client.keys().secret_key().to_secret_hex(),
        auth_tag: client.auth_tag_json().map(str::to_owned),
    }
}

fn execute_import(
    remotes: &RepoRemotes,
    auth: GitAuth,
    commit: &str,
) -> Result<WriteOutput, CliError> {
    let repo = GitRepo::new(auth)?;
    if repo.github_main(&remotes.github_url)? != commit {
        return Err(CliError::Conflict(
            "GitHub main does not equal --commit; no write was attempted".into(),
        ));
    }
    repo.fetch_github_main(&remotes.github_url, commit)?;
    let buzz_before = repo.ls_remote(&remotes.buzz_url, true)?;
    let changed = match buzz_before.main.as_deref() {
        None => {
            if buzz_before.head.is_some()
                || !matches!(buzz_before.head_target.as_deref(), None | Some(MAIN_REF))
            {
                return Err(CliError::Conflict(
                    "Buzz main is absent but remote HEAD is not empty and main-directed; no write was attempted"
                        .into(),
                ));
            }
            true
        }
        Some(current) if current == commit => {
            require_exact_head(&buzz_before, commit)?;
            false
        }
        Some(_) => {
            return Err(CliError::Conflict(
                "Buzz main is already populated at a different commit; use mirror-main with an exact lease"
                    .into(),
            ))
        }
    };
    if changed {
        if repo.github_main(&remotes.github_url)? != commit {
            return Err(CliError::Conflict(
                "GitHub main changed before the Buzz write; no write was attempted".into(),
            ));
        }
        repo.push_main(&remotes.buzz_url, commit, None)?;
    }
    let github_after = repo.github_main(&remotes.github_url)?;
    if github_after != commit {
        return Err(CliError::Conflict(
            "GitHub main changed during synchronization".into(),
        ));
    }
    let buzz_after = repo.ls_remote(&remotes.buzz_url, true)?;
    require_exact_head(&buzz_after, commit)?;
    let buzz_main = buzz_after
        .main
        .ok_or_else(|| CliError::Other("Buzz main missing after exact readback".into()))?;
    let buzz_head = buzz_after
        .head
        .ok_or_else(|| CliError::Other("Buzz HEAD missing after exact readback".into()))?;
    Ok(WriteOutput {
        repo_id: remotes.repo_id.clone(),
        direction: "github-to-buzz",
        commit: commit.to_owned(),
        changed,
        github_main: github_after,
        buzz_main,
        buzz_head,
    })
}

fn execute_mirror(
    remotes: &RepoRemotes,
    auth: GitAuth,
    commit: &str,
    expected_buzz_main: &str,
) -> Result<WriteOutput, CliError> {
    let repo = GitRepo::new(auth)?;
    if repo.github_main(&remotes.github_url)? != commit {
        return Err(CliError::Conflict(
            "GitHub main does not equal --commit; no write was attempted".into(),
        ));
    }
    repo.fetch_github_main(&remotes.github_url, commit)?;
    let buzz_before = repo.ls_remote(&remotes.buzz_url, true)?;
    if buzz_before.main.as_deref() != Some(expected_buzz_main) {
        return Err(CliError::Conflict(
            "Buzz main is absent or does not equal --expected-buzz-main; no write was attempted"
                .into(),
        ));
    }
    require_exact_head(&buzz_before, expected_buzz_main)?;
    repo.fetch_buzz_main(&remotes.buzz_url)?;
    if repo.rev_parse(BUZZ_TRACKING_REF)? != expected_buzz_main
        || !repo.is_ancestor(expected_buzz_main, commit)?
    {
        return Err(CliError::Conflict(
            "requested mirror is not a proven fast-forward from --expected-buzz-main".into(),
        ));
    }
    let changed = commit != expected_buzz_main;
    if changed {
        if repo.github_main(&remotes.github_url)? != commit {
            return Err(CliError::Conflict(
                "GitHub main changed before the Buzz write; no write was attempted".into(),
            ));
        }
        repo.push_main(&remotes.buzz_url, commit, Some(expected_buzz_main))?;
    }
    let github_after = repo.github_main(&remotes.github_url)?;
    if github_after != commit {
        return Err(CliError::Conflict(
            "GitHub main changed during synchronization".into(),
        ));
    }
    let buzz_after = repo.ls_remote(&remotes.buzz_url, true)?;
    require_exact_head(&buzz_after, commit)?;
    let buzz_main = buzz_after
        .main
        .ok_or_else(|| CliError::Other("Buzz main missing after exact readback".into()))?;
    let buzz_head = buzz_after
        .head
        .ok_or_else(|| CliError::Other("Buzz HEAD missing after exact readback".into()))?;
    Ok(WriteOutput {
        repo_id: remotes.repo_id.clone(),
        direction: "github-to-buzz",
        commit: commit.to_owned(),
        changed,
        github_main: github_after,
        buzz_main,
        buzz_head,
    })
}

pub async fn cmd_status(client: &BuzzClient, announcement: &Event) -> Result<(), CliError> {
    let remotes = derive_remotes(client, announcement)?;
    let repo = GitRepo::new(auth_from_client(client))?;
    let github_main = repo.github_main(&remotes.github_url)?;
    let buzz = repo.ls_remote(&remotes.buzz_url, true)?;
    let output = StatusOutput {
        repo_id: &remotes.repo_id,
        direction: "github-to-buzz",
        github_url: &remotes.github_url,
        buzz_url: &remotes.buzz_url,
        github_main: &github_main,
        buzz_main: buzz.main.as_deref(),
        buzz_head: buzz.head.as_deref(),
        in_sync: buzz.main.as_deref() == Some(github_main.as_str())
            && buzz.head.as_deref() == Some(github_main.as_str())
            && buzz.head_target.as_deref() == Some(MAIN_REF),
    };
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|_| CliError::Other("failed to serialize repository status".into()))?
    );
    Ok(())
}

pub async fn cmd_import_main(
    client: &BuzzClient,
    announcement: &Event,
    commit: &str,
) -> Result<(), CliError> {
    let commit = exact_oid(commit, "--commit")?;
    let remotes = derive_remotes(client, announcement)?;
    let output = execute_import(&remotes, auth_from_client(client), &commit)?;
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|_| CliError::Other("failed to serialize repository sync result".into()))?
    );
    Ok(())
}

pub async fn cmd_mirror_main(
    client: &BuzzClient,
    announcement: &Event,
    commit: &str,
    expected_buzz_main: &str,
) -> Result<(), CliError> {
    let commit = exact_oid(commit, "--commit")?;
    let expected = exact_oid(expected_buzz_main, "--expected-buzz-main")?;
    let remotes = derive_remotes(client, announcement)?;
    let output = execute_mirror(&remotes, auth_from_client(client), &commit, &expected)?;
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|_| CliError::Other("failed to serialize repository sync result".into()))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, Tag};
    use std::path::Path;

    fn git<I, S>(cwd: &Path, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("run fixture git");
        assert!(
            output.status.success(),
            "fixture git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    struct Fixture {
        _temp: TempDir,
        github: String,
        buzz: String,
        work: std::path::PathBuf,
        first: String,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().expect("fixture tempdir");
            let github = temp.path().join("github.git");
            let buzz = temp.path().join("buzz.git");
            let work = temp.path().join("work");
            git(temp.path(), ["init", "--bare", github.to_str().unwrap()]);
            git(temp.path(), ["init", "--bare", buzz.to_str().unwrap()]);
            git(&github, ["symbolic-ref", "HEAD", MAIN_REF]);
            git(&buzz, ["symbolic-ref", "HEAD", MAIN_REF]);
            git(temp.path(), ["init", work.to_str().unwrap()]);
            git(&work, ["config", "user.name", "Buzz Test"]);
            git(&work, ["config", "user.email", "buzz-test@example.invalid"]);
            git(&work, ["checkout", "-b", "main"]);
            std::fs::write(work.join("file.txt"), "one\n").expect("write fixture");
            git(&work, ["add", "file.txt"]);
            git(&work, ["commit", "-m", "one"]);
            let first = git(&work, ["rev-parse", "HEAD"]);
            git(
                &work,
                ["push", github.to_str().unwrap(), "HEAD:refs/heads/main"],
            );
            Self {
                github: github.to_string_lossy().into_owned(),
                buzz: buzz.to_string_lossy().into_owned(),
                work,
                first,
                _temp: temp,
            }
        }

        fn remotes(&self) -> RepoRemotes {
            RepoRemotes {
                repo_id: "fixture".into(),
                github_url: self.github.clone(),
                buzz_url: self.buzz.clone(),
            }
        }

        fn auth() -> GitAuth {
            GitAuth {
                private_key: "1".repeat(64),
                auth_tag: None,
            }
        }

        fn next_commit(&self) -> String {
            std::fs::write(self.work.join("file.txt"), "two\n").expect("write fixture");
            git(&self.work, ["add", "file.txt"]);
            git(&self.work, ["commit", "-m", "two"]);
            let commit = git(&self.work, ["rev-parse", "HEAD"]);
            git(
                &self.work,
                ["push", self.github.as_str(), "HEAD:refs/heads/main"],
            );
            commit
        }
    }

    #[test]
    fn exact_oid_requires_full_commit() {
        assert!(exact_oid(&"a".repeat(40), "--commit").is_ok());
        assert!(exact_oid(&"a".repeat(39), "--commit").is_err());
        assert!(exact_oid(&"z".repeat(40), "--commit").is_err());
    }

    #[test]
    fn announcement_requires_caller_owned_canonical_buzz_then_github() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let relay = "https://relay.example";
        let buzz = format!("{relay}/git/{owner}/fixture");
        let client = BuzzClient::new(relay.into(), keys.clone(), None, None).expect("client");
        let event = buzz_sdk::build_repo_announcement(
            "fixture",
            None,
            None,
            &[&buzz, "https://github.com/block/buzz.git"],
            None,
            &[],
        )
        .expect("announcement")
        .sign_with_keys(&keys)
        .expect("sign announcement");
        let remotes = derive_remotes(&client, &event).expect("canonical remotes");
        assert_eq!(remotes.buzz_url, buzz);
        assert_eq!(remotes.github_url, "https://github.com/block/buzz.git");

        let wrong_order = buzz_sdk::build_repo_announcement(
            "fixture",
            None,
            None,
            &["https://github.com/block/buzz.git", &buzz],
            None,
            &[],
        )
        .expect("announcement")
        .sign_with_keys(&keys)
        .expect("sign announcement");
        assert!(matches!(
            derive_remotes(&client, &wrong_order),
            Err(CliError::Usage(_))
        ));

        let duplicate_clone = nostr::EventBuilder::new(nostr::Kind::Custom(30617), "")
            .tags([
                Tag::parse(["d", "fixture"]).unwrap(),
                Tag::parse(["clone", &buzz, "https://github.com/block/buzz.git"]).unwrap(),
                Tag::parse(["clone", &buzz, "https://github.com/block/buzz.git"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign announcement");
        assert!(matches!(
            derive_remotes(&client, &duplicate_clone),
            Err(CliError::Usage(message)) if message.contains("multiple clone tags")
        ));
    }

    #[test]
    fn delegated_auth_owner_does_not_replace_repository_event_author() {
        let keys = Keys::generate();
        let auth_owner = Keys::generate().public_key().to_hex();
        let auth_tag = Tag::parse(["auth", auth_owner.as_str(), "", &"0".repeat(128)])
            .expect("syntactically valid auth tag");
        let relay = "https://relay.example";
        let repo_owner = keys.public_key().to_hex();
        let buzz = format!("{relay}/git/{repo_owner}/fixture");
        let client = BuzzClient::new(
            relay.into(),
            keys.clone(),
            Some(auth_tag),
            Some("[]".into()),
        )
        .expect("client");
        let event = buzz_sdk::build_repo_announcement(
            "fixture",
            None,
            None,
            &[&buzz, "https://github.com/block/buzz.git"],
            None,
            &[],
        )
        .expect("announcement")
        .sign_with_keys(&keys)
        .expect("sign announcement");

        let remotes = derive_remotes(&client, &event).expect("signing key owns repository");
        assert_eq!(remotes.buzz_url, buzz);
    }

    #[test]
    fn import_then_mirror_moves_only_main_by_fast_forward() {
        let fixture = Fixture::new();
        let imported = execute_import(&fixture.remotes(), Fixture::auth(), &fixture.first)
            .expect("import main");
        assert!(imported.changed);
        assert_eq!(
            git(Path::new(&fixture.buzz), ["show-ref"]),
            format!("{} refs/heads/main", fixture.first)
        );

        let second = fixture.next_commit();
        let mirrored = execute_mirror(&fixture.remotes(), Fixture::auth(), &second, &fixture.first)
            .expect("mirror main");
        assert!(mirrored.changed);
        assert_eq!(
            git(Path::new(&fixture.buzz), ["show-ref"]),
            format!("{second} refs/heads/main")
        );
    }

    #[test]
    fn import_refuses_populated_buzz_and_mirror_refuses_stale_lease() {
        let fixture = Fixture::new();
        git(
            &fixture.work,
            ["push", fixture.buzz.as_str(), "HEAD:refs/heads/main"],
        );
        let second = fixture.next_commit();
        assert!(matches!(
            execute_import(&fixture.remotes(), Fixture::auth(), &second),
            Err(CliError::Conflict(_))
        ));
        assert!(matches!(
            execute_mirror(
                &fixture.remotes(),
                Fixture::auth(),
                &second,
                &"f".repeat(40)
            ),
            Err(CliError::Conflict(_))
        ));
    }

    #[test]
    fn parse_remote_state_requires_exact_main_and_head() {
        let oid = "a".repeat(40);
        let state = parse_remote_state(&format!(
            "ref: refs/heads/main\tHEAD\n{oid}\tHEAD\n{oid}\trefs/heads/main\n"
        ))
        .expect("parse refs");
        assert_eq!(state.main.as_deref(), Some(oid.as_str()));
        assert_eq!(state.head.as_deref(), Some(oid.as_str()));
        assert_eq!(state.head_target.as_deref(), Some(MAIN_REF));
    }
}
