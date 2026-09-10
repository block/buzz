use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct CommandActions {
    current: Mutex<HashMap<String, Arc<()>>>,
}

impl CommandActions {
    pub(super) fn begin(&self, session: &str) -> Arc<()> {
        let ticket = Arc::new(());
        self.current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(session.to_owned(), Arc::clone(&ticket));
        ticket
    }

    // Hold the check through native dispatch: a newer request must not replace
    // the ticket between checking it and calling WebKit. Native callbacks must
    // not reenter this action tracker.
    pub(super) fn run<T>(
        &self,
        session: &str,
        ticket: &Arc<()>,
        operation: impl FnOnce() -> T,
    ) -> Option<T> {
        let current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !current
            .get(session)
            .is_some_and(|active| Arc::ptr_eq(active, ticket))
        {
            return None;
        }
        Some(operation())
    }

    pub(super) fn remove(&self, session: &str) {
        self.current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(session);
    }
}

#[cfg(test)]
mod tests {
    use super::CommandActions;
    use std::cell::Cell;

    #[test]
    fn a_delivered_response_cannot_dispatch_after_a_newer_action() {
        let actions = CommandActions::default();
        let old = actions.begin("session");
        let newest = actions.begin("session");
        let navigated = Cell::new(false);
        assert_eq!(actions.run("session", &old, || navigated.set(true)), None);
        assert!(!navigated.get());
        assert_eq!(
            actions.run("session", &newest, || "new address"),
            Some("new address")
        );
    }

    #[test]
    fn closing_a_session_invalidates_queued_dispatch_and_releases_state() {
        let actions = CommandActions::default();
        let ticket = actions.begin("session");
        actions.remove("session");
        assert_eq!(actions.run("session", &ticket, || "stale navigation"), None);
        assert!(actions.current.lock().unwrap().is_empty());
    }

    #[test]
    fn a_different_session_cannot_use_an_existing_ticket() {
        let actions = CommandActions::default();
        let ticket = actions.begin("first");
        assert_eq!(actions.run("second", &ticket, || true), None);
    }

    #[test]
    fn validation_and_dispatch_hold_the_same_lock() {
        let actions = CommandActions::default();
        let ticket = actions.begin("session");
        actions
            .run("session", &ticket, || {
                assert!(actions.current.try_lock().is_err());
            })
            .unwrap();
    }
}
