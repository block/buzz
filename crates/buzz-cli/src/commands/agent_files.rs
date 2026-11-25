//! `buzz agents export`, `import` and `edit`: a local agent as an `AGENT.md`
//! directory. The Buzz desktop reads the agent, and applies a change only
//! after the owner approves it there.

use std::path::{Path, PathBuf};

use nostr::PublicKey;
use serde_json::json;

use crate::agent_file::{self, codec, Action, AgentDocument, ParsedFile, ReadResult};
use crate::client::BuzzClient;
use crate::error::CliError;

pub const FILE_NAME: &str = "AGENT.md";
/// Front matter plus body, as the persona-pack limits allow.
const MAX_FILE_BYTES: u64 = 1024 * 1024 + 256 * 1024;

/// What to do with the directory's file.
pub struct ImportOptions {
    pub dry: bool,
    pub new: bool,
    pub workspace: Option<PathBuf>,
    pub prune_channels: bool,
}

/// `buzz agents export`.
pub async fn export(
    client: &BuzzClient,
    agent: &str,
    out: Option<PathBuf>,
    name: Option<String>,
    force: bool,
) -> Result<(), CliError> {
    let read = read_agent(client, agent).await?;
    let name = match name {
        Some(name) => {
            codec::check_name(&name).map_err(usage)?;
            name
        }
        // Import wants the name to match the directory, so --out names it.
        None => match out.as_deref().and_then(Path::file_name) {
            Some(dir) => {
                let dir = dir.to_string_lossy().into_owned();
                codec::check_name(&dir)
                    .map_err(|e| CliError::Usage(format!("{}; pass --name", e.0)))?;
                dir
            }
            None => agent_file::slug_name(&read.document.profile.name),
        },
    };
    let text = exportable(&read.document, &name)?;
    let dir = out.unwrap_or_else(|| PathBuf::from(&name));
    let path = dir.join(FILE_NAME);
    if path.exists() && !force {
        return Err(CliError::Usage(format!(
            "{} exists; pass --force to overwrite it",
            path.display()
        )));
    }
    std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
    std::fs::write(&path, text).map_err(|e| io(&path, e))?;
    println!(
        "{}",
        json!({
            "path": path.display().to_string(),
            "name": name,
            "agent": read.document.agent.as_ref().map(|a| a.pubkey.clone()),
        })
    );
    Ok(())
}

/// `buzz agents import`.
pub async fn import(
    client: &BuzzClient,
    dir: &Path,
    options: ImportOptions,
) -> Result<(), CliError> {
    let (path, parsed) = read_dir(dir)?;
    let target = if options.new {
        None
    } else {
        parsed.document.agent.as_ref().map(|a| a.pubkey.clone())
    };
    let live = match &target {
        Some(pubkey) => Some(read_agent(client, pubkey).await?),
        None => None,
    };
    let applied = apply(client, &parsed, live.as_ref(), &options).await?;
    if let Some(result) = applied {
        if target.is_none() && !options.new {
            write_back_identity(&path, &result)?;
        }
    }
    Ok(())
}

/// `buzz agents edit`: the live agent in `$VISUAL` or `$EDITOR`, then import.
pub async fn edit(client: &BuzzClient, agent: &str) -> Result<(), CliError> {
    let read = read_agent(client, agent).await?;
    let name = agent_file::slug_name(&read.document.profile.name);
    let text = exportable(&read.document, &name)?;
    let scratch = tempfile::tempdir().map_err(|e| CliError::Other(format!("temp dir: {e}")))?;
    let dir = scratch.path().join(&name);
    std::fs::create_dir(&dir).map_err(|e| io(&dir, e))?;
    let path = dir.join(FILE_NAME);
    let mut written = text;
    loop {
        std::fs::write(&path, &written).map_err(|e| io(&path, e))?;
        run_editor(&path)?;
        let saved = std::fs::read_to_string(&path).map_err(|e| io(&path, e))?;
        if saved == written {
            eprintln!("No changes saved; nothing to do.");
            return Ok(());
        }
        match codec::parse(&saved) {
            Ok(parsed) if parsed.name == name => {
                let options = ImportOptions {
                    dry: false,
                    new: false,
                    workspace: None,
                    prune_channels: false,
                };
                apply(client, &parsed, Some(&read), &options).await?;
                return Ok(());
            }
            Ok(parsed) => {
                written = with_error(
                    &saved,
                    &format!("name must stay {name}, not {}", parsed.name),
                );
            }
            Err(e) => written = with_error(&saved, &e.0),
        }
    }
}

/// Prints the plan, then, unless it's a dry run or there's nothing to do,
/// asks the desktop to apply it. Returns the desktop's result.
async fn apply(
    client: &BuzzClient,
    parsed: &ParsedFile,
    live: Option<&ReadResult>,
    options: &ImportOptions,
) -> Result<Option<serde_json::Value>, CliError> {
    let mut desired = parsed.document.clone();
    if let Some(live) = live {
        let (Some(have), Some(want)) = (&live.document.agent, &desired.agent) else {
            return Err(CliError::Other(
                "the Buzz desktop sent an agent document without its agent".into(),
            ));
        };
        if have.pubkey != want.pubkey {
            return Err(CliError::Other(format!(
                "asked Buzz for agent {} but it answered for {}",
                want.pubkey, have.pubkey
            )));
        }
        if want.community.is_some() && want.community != have.community {
            return Err(CliError::Usage(format!(
                "buzz.community is {}, but the agent lives in {}; an import can't move an agent",
                want.community.as_deref().unwrap_or_default(),
                have.community.as_deref().unwrap_or("its home community")
            )));
        }
        desired.agent = Some(agent_file::document::AgentRef {
            owner: None,
            ..have.clone()
        });
        // Without --prune-channels the import leaves channels the file omits,
        // so the plan keeps them too and shows only what will change.
        if !options.prune_channels {
            for kept in &live.document.channels {
                if !desired.channels.iter().any(|c| c.channel == kept.channel) {
                    desired.channels.push(kept.clone());
                }
            }
        }
    } else {
        desired.agent = None;
    }

    let plan =
        agent_file::plan(live.map(|l| &l.document), &desired, &parsed.name).map_err(usage)?;
    print!("{}", plan.report(&parsed.name));
    if options.dry || plan.action == Action::NoChange {
        return Ok(None);
    }

    let provider = desktop(client)?;
    let document = serde_json::to_string(&desired)
        .map_err(|e| CliError::Other(format!("could not encode the agent document: {e}")))?;
    let workspace = match &options.workspace {
        Some(path) => Some(
            std::fs::canonicalize(path)
                .map_err(|e| io(path, e))?
                .display()
                .to_string(),
        ),
        None => None,
    };
    let request = json!({"prune_channels": options.prune_channels, "workspace": workspace});
    let (agent, revision) = match live {
        Some(live) => (
            desired
                .agent
                .as_ref()
                .map(|a| a.pubkey.clone())
                .unwrap_or_default(),
            live.revision.clone(),
        ),
        None => (String::new(), String::new()),
    };
    eprintln!("Approve the change in Buzz to apply it.");
    let answer = provider
        .agent_apply(&agent, &document, &revision, &request.to_string())
        .await?;
    let result: serde_json::Value = serde_json::from_str(&answer)
        .map_err(|e| CliError::Other(format!("the Buzz desktop sent an invalid result: {e}")))?;
    println!("{result}");
    Ok(Some(result))
}

/// Renders the agent and checks that the file reads back as the same agent.
fn exportable(doc: &AgentDocument, name: &str) -> Result<String, CliError> {
    let text = agent_file::render(doc, name).map_err(usage)?;
    let parsed = codec::parse(&text)
        .map_err(|e| CliError::Other(format!("the exported file doesn't read back: {}", e.0)))?;
    if let Action::Update(fields) = agent_file::plan(Some(doc), &parsed.document, name)
        .map_err(usage)?
        .action
    {
        return Err(CliError::Other(format!(
            "the exported file would change {}; this is a bug in buzz",
            fields.join(", ")
        )));
    }
    Ok(text)
}

async fn read_agent(client: &BuzzClient, agent: &str) -> Result<ReadResult, CliError> {
    // An npub or hex key goes as hex; anything else is a name for the desktop
    // to resolve among the owner's agents.
    let agent = match PublicKey::parse(agent) {
        Ok(key) => key.to_hex(),
        Err(_) => agent.to_owned(),
    };
    let answer = desktop(client)?.agent_read(&agent).await?;
    serde_json::from_str(&answer).map_err(|e| {
        CliError::Other(format!(
            "the Buzz desktop sent an agent this buzz can't read ({e}); update buzz"
        ))
    })
}

/// Reads `AGENT.md` from a directory, or the file itself, and checks that
/// its name matches the directory.
fn read_dir(dir: &Path) -> Result<(PathBuf, ParsedFile), CliError> {
    let path = if dir.is_dir() {
        dir.join(FILE_NAME)
    } else {
        dir.to_path_buf()
    };
    let size = std::fs::metadata(&path).map_err(|e| io(&path, e))?.len();
    if size > MAX_FILE_BYTES {
        return Err(CliError::Usage(format!("{} is too large", path.display())));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| io(&path, e))?;
    let parsed =
        codec::parse(&text).map_err(|e| CliError::Usage(format!("{}: {}", path.display(), e.0)))?;
    let folder = std::fs::canonicalize(&path).ok().and_then(|p| {
        p.parent()
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    });
    if let Some(folder) = folder {
        if folder != parsed.name {
            return Err(CliError::Usage(format!(
                "{}: name is {}, but the directory is {folder}",
                path.display(),
                parsed.name
            )));
        }
    }
    Ok((path, parsed))
}

/// Records a new agent's npub in the file it was created from, keeping the
/// rest of the file as written.
fn write_back_identity(path: &Path, result: &serde_json::Value) -> Result<(), CliError> {
    let Some(pubkey) = result.get("agent").and_then(|a| a.as_str()) else {
        return Err(CliError::Other(
            "the Buzz desktop created the agent but didn't say which one".into(),
        ));
    };
    let npub = codec::npub(pubkey).map_err(|e| CliError::Other(e.0))?;
    let text = std::fs::read_to_string(path).map_err(|e| io(path, e))?;
    let Some(updated) = insert_identity(&text, &npub) else {
        eprintln!(
            "warning: add `identity: {{buzz: {npub}}}` to {} so the next import updates this agent",
            path.display()
        );
        return Ok(());
    };
    std::fs::write(path, updated).map_err(|e| io(path, e))?;
    eprintln!("Wrote identity.buzz: {npub} to {}", path.display());
    Ok(())
}

/// The file with `identity.buzz` added after the top-level `name:` line, or
/// `None` if the front matter already has an `identity` key.
fn insert_identity(text: &str, npub: &str) -> Option<String> {
    let front_end = text.get(4..)?.find("\n---")? + 4;
    let front = &text[..front_end];
    if front.lines().any(|l| l.starts_with("identity:")) {
        return None;
    }
    let mut out = String::with_capacity(text.len() + 80);
    let mut done = false;
    for line in text.split_inclusive('\n') {
        out.push_str(line);
        if !done && out.len() <= front_end + 1 && line.starts_with("name:") {
            out.push_str(&format!("identity:\n  buzz: {npub}\n"));
            done = true;
        }
    }
    done.then_some(out)
}

/// The file with an error noted as comments at the top of its front matter.
fn with_error(saved: &str, error: &str) -> String {
    let body = saved.strip_prefix("---\n").unwrap_or(saved);
    let body: String = body
        .split_inclusive('\n')
        .skip_while(|l| l.starts_with("# buzz: "))
        .collect();
    let notes: String = error.lines().map(|l| format!("# buzz: {l}\n")).collect();
    format!("---\n{notes}# buzz: fix this and save, or quit without saving to cancel\n{body}")
}

fn run_editor(path: &Path) -> Result<(), CliError> {
    // An empty $VISUAL falls through to $EDITOR, as in git.
    let set = |name| {
        std::env::var(name)
            .ok()
            .filter(|e: &String| !e.trim().is_empty())
    };
    let editor = set("VISUAL")
        .or_else(|| set("EDITOR"))
        .unwrap_or_else(|| "vi".to_owned());
    // Through the shell, so an editor with arguments, such as `code --wait`,
    // works as it does for git.
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("buzz-agents-edit")
        .arg(path)
        .status()
        .map_err(|e| CliError::Other(format!("could not start {editor}: {e}")))?;
    if !status.success() {
        return Err(CliError::Other(format!(
            "{editor} exited with {status}; nothing changed"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn desktop(client: &BuzzClient) -> Result<&crate::desktop_provider::Provider, CliError> {
    client.credentials().owner_desktop()
}

#[cfg(not(unix))]
fn desktop(_: &BuzzClient) -> Result<&NoDesktop, CliError> {
    Err(CliError::Usage(
        "buzz agents export and import need the Buzz desktop, which this platform's buzz can't reach yet".into(),
    ))
}

#[cfg(not(unix))]
pub enum NoDesktop {}

#[cfg(not(unix))]
impl NoDesktop {
    async fn agent_read(&self, _: &str) -> Result<String, CliError> {
        match *self {}
    }
    async fn agent_apply(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String, CliError> {
        match *self {}
    }
}

fn usage(e: agent_file::CodecError) -> CliError {
    CliError::Usage(e.0)
}

fn io(path: &Path, e: std::io::Error) -> CliError {
    CliError::Usage(format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_created_agent_is_written_back_after_its_name() {
        let text = "---\n# mine\nkind: Agent\nname: a\ndescription: d\n---\nBody name: x\n";
        assert_eq!(
            insert_identity(text, "npub1x").unwrap(),
            "---\n# mine\nkind: Agent\nname: a\nidentity:\n  buzz: npub1x\ndescription: d\n---\nBody name: x\n"
        );
        assert_eq!(
            insert_identity("---\nidentity: {}\nname: a\n---\n", "npub1x"),
            None
        );
    }

    #[test]
    fn an_edit_that_does_not_parse_reopens_with_the_error() {
        let once = with_error("---\nkind: Agent\n---\n", "first");
        let twice = with_error(&once, "second\nline");
        assert_eq!(
            twice,
            "---\n# buzz: second\n# buzz: line\n# buzz: fix this and save, or quit without saving to cancel\nkind: Agent\n---\n"
        );
    }
}
