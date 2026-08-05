use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use reqwest::header::RETRY_AFTER;
use url::Url;

pub(super) const MAX_IMAGE_RETRY_AFTER: Duration = Duration::from_secs(60 * 60);
const MAX_IMAGE_HOST_COOLDOWNS: usize = 128;

static IMAGE_HOST_COOLDOWNS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

pub(super) fn retry_after_duration(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .map(|duration| duration.min(MAX_IMAGE_RETRY_AFTER))
}

pub(super) fn image_host_cooldown_remaining(url: &Url) -> Option<Duration> {
    let host = url.host_str()?;
    let cooldowns = IMAGE_HOST_COOLDOWNS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut cooldowns) = cooldowns.lock() else {
        return None;
    };
    let expires_at = cooldowns.get(host).copied()?;
    let now = Instant::now();
    if expires_at <= now {
        cooldowns.remove(host);
        return None;
    }
    Some(expires_at.duration_since(now))
}

pub(super) fn set_image_host_cooldown(url: &Url, retry_after: Duration) {
    let Some(host) = url.host_str() else {
        return;
    };
    let now = Instant::now();
    let Some(expires_at) = now.checked_add(retry_after) else {
        return;
    };
    let cooldowns = IMAGE_HOST_COOLDOWNS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut cooldowns) = cooldowns.lock() {
        cooldowns.retain(|_, current_expiry| *current_expiry > now);
        if cooldowns.len() >= MAX_IMAGE_HOST_COOLDOWNS && !cooldowns.contains_key(host) {
            let oldest_host = cooldowns
                .iter()
                .min_by_key(|(_, current_expiry)| *current_expiry)
                .map(|(current_host, _)| current_host.clone());
            if let Some(oldest_host) = oldest_host {
                cooldowns.remove(&oldest_host);
            }
        }
        cooldowns.insert(host.to_string(), expires_at);
    }
}
