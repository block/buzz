//! One-time challenges. The browser's remote signer signs a kind-22242 event that carries the challenge, this
//! desktop's id, the connection id and the purpose; the desktop verifies it against its own member key.
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use nostr::{Event, PublicKey};
use serde::Serialize;

pub const AUTH_KIND: u16 = 22242;
pub const CHALLENGE_TTL_SECS: u64 = 60;
const MAX_SKEW_SECS: u64 = 60;
const MAX_OPEN: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    Connect,
    /// A fresh signature is required for every terminal launch; `launch_id` is chosen by the browser.
    Terminal(String),
}

impl Purpose {
    pub fn parse(s: &str) -> Option<Purpose> {
        if s == "connect" {
            return Some(Purpose::Connect);
        }
        let id = s.strip_prefix("terminal:")?;
        let ok = (8..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        ok.then(|| Purpose::Terminal(id.to_string()))
    }
    pub fn as_tag(&self) -> String {
        match self {
            Purpose::Connect => "connect".into(),
            Purpose::Terminal(id) => format!("terminal:{id}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub nonce: String,
    pub exp: u64,
    pub conn_id: String,
    pub desktop_id: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChallengeError {
    BadEvent,
    BadSignature,
    WrongKind,
    WrongMember,
    Stale,
    UnknownChallenge,
    Expired,
    WrongConnection,
    WrongDesktop,
    WrongPurpose,
}

impl ChallengeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadEvent => "bad_event",
            Self::BadSignature => "bad_signature",
            Self::WrongKind => "wrong_kind",
            Self::WrongMember => "wrong_member",
            Self::Stale => "stale",
            Self::UnknownChallenge => "unknown_challenge",
            Self::Expired => "expired",
            Self::WrongConnection => "wrong_connection",
            Self::WrongDesktop => "wrong_desktop",
            Self::WrongPurpose => "wrong_purpose",
        }
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::getrandom(&mut b).expect("os entropy");
    hex::encode(b)
}

pub struct ChallengeBook {
    member: PublicKey,
    desktop_id: String,
    open: HashMap<String, Challenge>,
}

fn tag_value<'a>(ev: &'a Event, name: &str) -> Option<&'a str> {
    ev.tags
        .iter()
        .find_map(|t| {
            let s = t.as_slice();
            (s.first().map(String::as_str) == Some(name)).then(|| s.get(1).map(String::as_str)).flatten()
        })
}

impl ChallengeBook {
    pub fn new(member: PublicKey, desktop_id: impl Into<String>) -> Self {
        Self { member, desktop_id: desktop_id.into(), open: HashMap::new() }
    }

    pub fn issue(&mut self, conn_id: &str, now: u64) -> Challenge {
        self.open.retain(|_, c| c.exp > now);
        if self.open.len() >= MAX_OPEN {
            if let Some(k) = self.open.keys().next().cloned() {
                self.open.remove(&k);
            }
        }
        let c = Challenge {
            nonce: random_hex(32),
            exp: now + CHALLENGE_TTL_SECS,
            conn_id: conn_id.to_string(),
            desktop_id: self.desktop_id.clone(),
        };
        self.open.insert(c.nonce.clone(), c.clone());
        c
    }

    /// Verifies a signed event for `conn_id` and returns the purpose it authorizes. The challenge is consumed
    /// whether or not verification succeeds past the lookup, so a signature can never be tried twice.
    pub fn verify(&mut self, event_json: &str, conn_id: &str, now: u64) -> Result<Purpose, ChallengeError> {
        let ev: Event = serde_json::from_str(event_json).map_err(|_| ChallengeError::BadEvent)?;
        if ev.kind.as_u16() != AUTH_KIND {
            return Err(ChallengeError::WrongKind);
        }
        ev.verify().map_err(|_| ChallengeError::BadSignature)?;
        if ev.pubkey != self.member {
            return Err(ChallengeError::WrongMember);
        }
        let ts = ev.created_at.as_secs();
        if ts.abs_diff(now) > MAX_SKEW_SECS {
            return Err(ChallengeError::Stale);
        }
        let nonce = tag_value(&ev, "challenge").ok_or(ChallengeError::UnknownChallenge)?;
        let ch = self.open.remove(nonce).ok_or(ChallengeError::UnknownChallenge)?; // single use
        if now > ch.exp {
            return Err(ChallengeError::Expired);
        }
        if ch.conn_id != conn_id || tag_value(&ev, "conn") != Some(conn_id) {
            return Err(ChallengeError::WrongConnection);
        }
        if ch.desktop_id != self.desktop_id || tag_value(&ev, "desktop") != Some(self.desktop_id.as_str()) {
            return Err(ChallengeError::WrongDesktop);
        }
        Purpose::parse(tag_value(&ev, "purpose").unwrap_or("")).ok_or(ChallengeError::WrongPurpose)
    }

    pub fn drop_connection(&mut self, conn_id: &str) {
        self.open.retain(|_, c| c.conn_id != conn_id);
    }
}
