use std::sync::Arc;
use std::time::Duration;

use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

pub(crate) struct CommandAdviserBackgroundServices {
    memory_sync: Option<Arc<crate::command_services::memory::MemorySyncScheduler>>,
    model_readiness: crate::startup::CommandBriefModelReadinessObserver,
    knowledge_refresh_cancellation: CancellationToken,
}

impl CommandAdviserBackgroundServices {
    pub(crate) fn start(app: AppHandle) -> Self {
        let memory_sync = crate::command_services::memory::start_memory_sync_scheduler(app.clone());
        let model_readiness = crate::startup::start_model_readiness_observer(app.clone());
        #[cfg(target_os = "macos")]
        let _ = crate::startup::install_system_wake_source(app.clone());

        let startup_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::startup::run_command_brief_schedule(
                startup_app,
                crate::command_brief::schedule::ScheduleTrigger::Startup,
            )
            .await;
        });

        let knowledge_refresh_cancellation = CancellationToken::new();
        let task_cancellation = knowledge_refresh_cancellation.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                crate::command_services::policy::status::refresh_knowledge_admissions(app.clone())
                    .await;
                let _ = crate::startup::run_command_brief_schedule(
                    app.clone(),
                    crate::command_brief::schedule::ScheduleTrigger::Timer,
                )
                .await;
                tokio::select! {
                    () = task_cancellation.cancelled() => break,
                    () = tokio::time::sleep(Duration::from_secs(15)) => {}
                }
            }
        });

        Self {
            memory_sync,
            model_readiness,
            knowledge_refresh_cancellation,
        }
    }

    pub(crate) fn stop(&self) {
        crate::command_services::memory::cancel_active_memory_sync();
        self.model_readiness.stop();
        self.knowledge_refresh_cancellation.cancel();
        if let Some(scheduler) = &self.memory_sync {
            let _ = scheduler.stop_and_join();
        }
    }
}

pub(crate) fn enable_autostart(app: &tauri::App) {
    if !cfg!(debug_assertions) {
        use tauri_plugin_autostart::ManagerExt;
        if let Err(error) = app.autolaunch().enable() {
            eprintln!("command-adviser: could not enable start at login: {error}");
        }
    }
}

pub(crate) fn migrate_command_team_parallelism(app: &AppHandle) {
    match crate::managed_agents::migrate_command_adviser_parallelism(app) {
        Ok(changed) if changed > 0 => eprintln!(
            "buzz-desktop: migrated {changed} Command Team agent(s) to single-turn parallelism"
        ),
        Ok(_) => {}
        Err(error) => {
            eprintln!("buzz-desktop: Command Team parallelism migration failed: {error}");
        }
    }
}
