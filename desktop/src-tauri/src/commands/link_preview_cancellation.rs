use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct LinkPreviewCancellations {
    tokens: HashMap<String, CancellationToken>,
}

impl LinkPreviewCancellations {
    fn begin(&mut self, request_id: &str) -> CancellationToken {
        if let Some(cancellation) = self.tokens.get(request_id).cloned() {
            return cancellation;
        }
        let cancellation = CancellationToken::new();
        self.tokens
            .insert(request_id.to_string(), cancellation.clone());
        cancellation
    }

    fn cancel(&mut self, request_id: &str) {
        self.tokens
            .entry(request_id.to_string())
            .or_default()
            .cancel();
    }

    fn finish(&mut self, request_id: &str) {
        self.tokens.remove(request_id);
    }
}

static LINK_PREVIEW_CANCELLATIONS: LazyLock<Mutex<LinkPreviewCancellations>> =
    LazyLock::new(|| Mutex::new(LinkPreviewCancellations::default()));

pub(super) fn begin(request_id: Option<&str>) -> Option<CancellationToken> {
    let request_id = request_id?;
    LINK_PREVIEW_CANCELLATIONS
        .lock()
        .ok()
        .map(|mut fetches| fetches.begin(request_id))
}

pub(super) fn cancel(request_id: &str) {
    if let Ok(mut fetches) = LINK_PREVIEW_CANCELLATIONS.lock() {
        fetches.cancel(request_id);
    }
}

pub(super) fn finish(request_id: Option<&str>) {
    let Some(request_id) = request_id else {
        return;
    };
    if let Ok(mut fetches) = LINK_PREVIEW_CANCELLATIONS.lock() {
        fetches.finish(request_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_before_begin_is_retained() {
        let mut fetches = LinkPreviewCancellations::default();
        fetches.cancel("cancel-before-begin");

        let cancellation = fetches.begin("cancel-before-begin");

        assert!(cancellation.is_cancelled());
        fetches.finish("cancel-before-begin");
        assert!(fetches.tokens.is_empty());
    }

    #[test]
    fn cancellation_reaches_active_owner() {
        let mut fetches = LinkPreviewCancellations::default();
        let cancellation = fetches.begin("active-fetch");

        fetches.cancel("active-fetch");

        assert!(cancellation.is_cancelled());
        fetches.finish("active-fetch");
        assert!(fetches.tokens.is_empty());
    }
}
