//! buzz-agent's system prompt, assembled with goose's `PromptManager`.
//!
//! `Agent::prompt_manager` is `pub(super)` in goose, and `Agent::reply` is the
//! only thing that reads it. Since buzz-agent owns the loop it also owns the
//! prompt: we keep our own `PromptManager` and call it at the top of each
//! round, which is exactly what goose's inference step does internally.
//!
//! Owning it is not just a workaround for the visibility — it is the seam that
//! makes the persona work at all. goose's ACP server never reads
//! `systemPrompt`, so an agent driven over plain ACP silently loses Fizz and
//! the `[Base]` operating manual. Building the prompt here puts them in front
//! of the model.

use std::path::Path;
use std::sync::Arc;

use goose::agents::PromptManager;
use goose_provider_types::goose_mode::GooseMode;
use tokio::sync::Mutex;

/// The system prompt for one session.
///
/// Cloneable so a turn can hold it without borrowing the session map.
#[derive(Clone)]
pub struct SessionPrompt {
    manager: Arc<Mutex<PromptManager>>,
    mode: GooseMode,
}

impl SessionPrompt {
    pub fn new(mode: GooseMode) -> Self {
        Self {
            manager: Arc::new(Mutex::new(PromptManager::new())),
            mode,
        }
    }

    /// Replace goose's base template with the harness-supplied persona.
    pub async fn set_override(&self, template: String) {
        self.manager
            .lock()
            .await
            .set_system_prompt_override(template);
    }

    /// Append a keyed section (AGENTS.md hints, the skill index, hook
    /// guidance). Extras survive an override — goose appends them to whichever
    /// base template is in force.
    pub async fn add_extra(&self, key: &str, instruction: String) {
        self.manager
            .lock()
            .await
            .add_system_prompt_extra(key.to_string(), instruction);
    }

    /// Render the prompt for one inference.
    ///
    /// Called per round rather than once per session because
    /// `build_system_prompt` also folds in subdirectory hints discovered from
    /// the tool arguments seen so far — a prompt built once at session start
    /// would never pick them up.
    pub async fn build(&self, working_dir: &Path) -> String {
        self.manager
            .lock()
            .await
            .build_system_prompt(working_dir, Vec::new(), self.mode)
    }

    /// Feed tool arguments to the subdirectory-hint tracker, so entering a new
    /// part of the tree pulls in that directory's AGENTS.md on the next round.
    pub async fn record_tool_arguments(
        &self,
        arguments: &Option<serde_json::Map<String, serde_json::Value>>,
        working_dir: &Path,
    ) {
        self.manager
            .lock()
            .await
            .record_tool_arguments(arguments, working_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn override_replaces_the_base_template() {
        let prompt = SessionPrompt::new(GooseMode::Auto);
        prompt.set_override("You are Fizz.".to_string()).await;
        let built = prompt.build(Path::new(".")).await;
        assert!(built.contains("You are Fizz."));
    }

    #[tokio::test]
    async fn extras_survive_an_override() {
        // The persona arrives as an override and the hints as an extra; losing
        // either one is a silent failure the harness cannot detect.
        let prompt = SessionPrompt::new(GooseMode::Auto);
        prompt.set_override("You are Fizz.".to_string()).await;
        prompt
            .add_extra("buzz_hints", "Always ship the sprocket first.".to_string())
            .await;
        let built = prompt.build(Path::new(".")).await;
        assert!(built.contains("You are Fizz."));
        assert!(built.contains("Always ship the sprocket first."));
    }
}
