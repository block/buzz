//! Tailscale integration. The user manages Tailscale: this module only DETECTS it and, when asked, adds/removes
//! one `tailscale serve` entry on a single https port that this app owns. It never installs, starts, signs in,
//! resets other serve config, or uses funnel.
use std::path::PathBuf;
use std::process::Stdio;

use serde_json::Value;
use tokio::process::Command;

/// The tailnet https port this app publishes on (the user's own serve entries on other ports are untouched).
pub const SERVE_PORT: u16 = 9443;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailscaleStatus {
    pub up: bool,
    pub dns_name: Option<String>,
    pub addrs: Vec<String>,
    /// Human reason shown next to the disabled switch.
    pub reason: Option<String>,
}

impl TailscaleStatus {
    pub fn down(reason: impl Into<String>) -> Self {
        Self { up: false, dns_name: None, addrs: vec![], reason: Some(reason.into()) }
    }
}

/// Parses `tailscale status --json`. "Up" means BackendState == Running and a tailnet address exists.
pub fn parse_status(json: &str) -> TailscaleStatus {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return TailscaleStatus::down("Couldn't read Tailscale's status.");
    };
    let state = v.get("BackendState").and_then(Value::as_str).unwrap_or("");
    if state != "Running" {
        return TailscaleStatus::down(match state {
            "NeedsLogin" | "NeedsMachineAuth" => "Tailscale needs you to sign in on this Mac.",
            "Stopped" => "Tailscale is turned off on this Mac.",
            _ => "Tailscale isn't running on this Mac.",
        });
    }
    let me = v.get("Self");
    let addrs: Vec<String> = me
        .and_then(|s| s.get("TailscaleIPs"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let dns = me
        .and_then(|s| s.get("DNSName"))
        .and_then(Value::as_str)
        .map(|s| s.trim_end_matches('.').to_string())
        .filter(|s| !s.is_empty());
    if addrs.is_empty() {
        return TailscaleStatus::down("This Mac has no tailnet address yet.");
    }
    TailscaleStatus { up: true, dns_name: dns, addrs, reason: None }
}

/// True if `serve status --json` has an entry for our port that proxies to loopback (i.e. one we created).
pub fn owns_entry(serve_json: &str, port: u16) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(serve_json) else { return false };
    let Some(web) = v.get("Web").and_then(Value::as_object) else { return false };
    web.iter().any(|(host, cfg)| {
        host.ends_with(&format!(":{port}"))
            && cfg
                .get("Handlers")
                .and_then(Value::as_object)
                .map(|h| h.values().any(|x| x.get("Proxy").and_then(Value::as_str).map_or(false, |p| p.contains("127.0.0.1") || p.contains("localhost"))))
                .unwrap_or(false)
    })
}

#[allow(async_fn_in_trait)]
pub trait TailscaleCli: Send + Sync {
    async fn status(&self) -> TailscaleStatus;
    async fn serve_status_json(&self) -> Option<String>;
    async fn serve_on(&self, https_port: u16, local_port: u16) -> Result<(), String>;
    async fn serve_off(&self, https_port: u16) -> Result<(), String>;
}

pub struct SystemTailscale;

impl SystemTailscale {
    fn bin() -> Option<PathBuf> {
        ["/usr/local/bin/tailscale", "/opt/homebrew/bin/tailscale", "/Applications/Tailscale.app/Contents/MacOS/Tailscale"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
            .or_else(|| std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("tailscale")).find(|c| c.is_file())))
    }
    async fn run(args: &[&str]) -> Result<String, String> {
        let bin = Self::bin().ok_or("Tailscale isn't installed on this Mac.")?;
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Command::new(bin).args(args).stdin(Stdio::null()).kill_on_drop(true).output(),
        )
        .await
        .map_err(|_| "Tailscale didn't answer in time.".to_string())?
        .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
}

impl TailscaleCli for SystemTailscale {
    async fn status(&self) -> TailscaleStatus {
        match Self::run(&["status", "--json"]).await {
            Ok(j) => parse_status(&j),
            Err(e) => TailscaleStatus::down(e),
        }
    }
    async fn serve_status_json(&self) -> Option<String> {
        Self::run(&["serve", "status", "--json"]).await.ok()
    }
    async fn serve_on(&self, https_port: u16, local_port: u16) -> Result<(), String> {
        let https = format!("--https={https_port}");
        let target = format!("http://127.0.0.1:{local_port}");
        Self::run(&["serve", "--bg", &https, &target]).await.map(|_| ())
    }
    async fn serve_off(&self, https_port: u16) -> Result<(), String> {
        let https = format!("--https={https_port}");
        Self::run(&["serve", &https, "off"]).await.map(|_| ())
    }
}

/// Owns the serve entry's lifecycle: stale cleanup before enabling, removal on disable/quit, auto-off when
/// Tailscale goes down. `enabled` is the user's switch; it turns itself off and stays off after a drop.
pub struct ServeManager<T: TailscaleCli> {
    cli: T,
    port: u16,
    enabled: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TickOutcome {
    Unchanged,
    /// Tailscale went down: the switch was turned off and the serve entry removed.
    TurnedOff,
}

impl<T: TailscaleCli> ServeManager<T> {
    pub fn new(cli: T) -> Self {
        Self { cli, port: SERVE_PORT, enabled: false }
    }
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub async fn status(&self) -> TailscaleStatus {
        self.cli.status().await
    }
    /// Removes a leftover entry from a previous run (crash). Only our own port, only if it proxies to loopback.
    pub async fn cleanup_stale(&self) {
        if let Some(j) = self.cli.serve_status_json().await {
            if owns_entry(&j, self.port) {
                let _ = self.cli.serve_off(self.port).await;
            }
        }
    }
    /// Turns the switch on. Refuses unless Tailscale is already up; never brings it up.
    pub async fn enable(&mut self, local_port: u16) -> Result<TailscaleStatus, String> {
        let st = self.cli.status().await;
        if !st.up {
            return Err(st.reason.clone().unwrap_or_else(|| "Tailscale isn't running on this Mac.".into()));
        }
        self.cleanup_stale().await;
        self.cli.serve_on(self.port, local_port).await?;
        self.enabled = true;
        Ok(st)
    }
    pub async fn disable(&mut self) {
        if self.enabled {
            let _ = self.cli.serve_off(self.port).await;
        }
        self.enabled = false;
    }
    /// Call every ~10 s while enabled.
    pub async fn tick(&mut self) -> TickOutcome {
        if self.enabled && !self.cli.status().await.up {
            self.disable().await;
            return TickOutcome::TurnedOff;
        }
        TickOutcome::Unchanged
    }
}
