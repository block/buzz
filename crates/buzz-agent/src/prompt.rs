//! Per-session system prompt composition using the Goose GDK.

use std::path::Path;
use std::sync::Arc;

use goose_agent::prompt::{
    InstructionDiscovery, InstructionDiscoveryOptions, PromptComposer, PromptContext, PromptSource,
};
use goose_provider_types::goose_mode::GooseMode;
use tokio::sync::Mutex;

struct PromptState {
    composer: PromptComposer,
    instructions: InstructionDiscovery,
    root_instructions_loaded: bool,
}

/// The system prompt for one session.
#[derive(Clone)]
pub struct SessionPrompt {
    state: Arc<Mutex<PromptState>>,
    mode: GooseMode,
}

impl SessionPrompt {
    pub fn new(mode: GooseMode) -> Self {
        Self {
            state: Arc::new(Mutex::new(PromptState {
                composer: PromptComposer::new(PromptSource::Literal(String::new())),
                instructions: InstructionDiscovery::new(InstructionDiscoveryOptions::default()),
                root_instructions_loaded: false,
            })),
            mode,
        }
    }

    /// Replace the base prompt with the harness-supplied persona template.
    pub async fn set_override(&self, template: String) {
        self.state
            .lock()
            .await
            .composer
            .set_base(PromptSource::Template(template));
    }

    /// Append or replace a keyed prompt section.
    pub async fn add_extra(&self, key: &str, instruction: String) {
        self.state
            .lock()
            .await
            .composer
            .add_extra(key.to_string(), instruction);
    }

    /// Render the prompt for one inference.
    pub async fn build(&self, working_dir: &Path) -> anyhow::Result<String> {
        let mut state = self.state.lock().await;
        if !state.root_instructions_loaded {
            for instruction in state.instructions.discover_root(working_dir)? {
                state
                    .composer
                    .add_extra(instruction.key, instruction.content);
            }
            state.root_instructions_loaded = true;
        }
        for instruction in state
            .instructions
            .discover_new_subdirectory_instructions(working_dir)?
        {
            state
                .composer
                .add_extra(instruction.key, instruction.content);
        }
        state.composer.render(
            &PromptContext {
                current_date_time: String::new(),
                goose_mode: self.mode,
                variables: Default::default(),
            },
            [],
        )
    }

    /// Feed tool arguments to the subdirectory-instruction tracker.
    pub async fn record_tool_arguments(
        &self,
        arguments: &Option<serde_json::Map<String, serde_json::Value>>,
        working_dir: &Path,
    ) {
        self.state
            .lock()
            .await
            .instructions
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
        let built = prompt.build(Path::new(".")).await.unwrap();
        assert!(built.contains("You are Fizz."));
    }

    #[tokio::test]
    async fn extras_survive_an_override() {
        let prompt = SessionPrompt::new(GooseMode::Auto);
        prompt.set_override("You are Fizz.".to_string()).await;
        prompt
            .add_extra("buzz_hints", "Always ship the sprocket first.".to_string())
            .await;
        let built = prompt.build(Path::new(".")).await.unwrap();
        assert!(built.contains("You are Fizz."));
        assert!(built.contains("Always ship the sprocket first."));
    }

    #[tokio::test]
    async fn instructions_are_loaded_once() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("AGENTS.md"), "Project instruction").unwrap();
        let prompt = SessionPrompt::new(GooseMode::Auto);
        prompt.set_override("Persona".to_string()).await;
        let first = prompt.build(root.path()).await.unwrap();
        let second = prompt.build(root.path()).await.unwrap();
        assert_eq!(first.matches("Project instruction").count(), 1);
        assert_eq!(second.matches("Project instruction").count(), 1);
    }
}
