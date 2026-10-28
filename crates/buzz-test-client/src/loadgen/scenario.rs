//! Scenario file schema and derived fixture layout.

use serde::Deserialize;

use super::identity;

/// A load scenario, parsed from TOML.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Seeds every derived identity, host and channel id. Change it for a fresh fixture.
    pub run_id: String,
    pub target: Target,
    pub shape: Shape,
    pub population: Population,
    #[serde(default)]
    pub human: HumanProfile,
    #[serde(default)]
    pub agent: AgentProfile,
    pub ramp: Vec<RampStep>,
    #[serde(default)]
    pub abort: Abort,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// WebSocket URL of the relay (single community) or the scheme source for
    /// templated community hosts, e.g. `ws://localhost:3000`.
    pub ws_url: String,
    /// HTTP base URL matching `ws_url`, e.g. `http://localhost:3000`.
    pub http_url: String,
    /// Community host template with `{run}` and `{i}` placeholders. Unset means
    /// one community at `ws_url`'s host.
    #[serde(default)]
    pub community_host_template: Option<String>,
    /// Open TCP connections here (`host:port`) instead of resolving community
    /// hosts, e.g. the relay's in-cluster service. Host headers still name the community.
    #[serde(default)]
    pub connect_addr: Option<String>,
    /// Deployment-root HTTP URL serving `POST /operator/communities`.
    #[serde(default)]
    pub operator_url: Option<String>,
    /// Environment variable holding the operator's hex secret key.
    #[serde(default = "default_operator_key_env")]
    pub operator_key_env: String,
}

fn default_operator_key_env() -> String {
    "LOADGEN_OPERATOR_KEY".into()
}

/// Data-shape vectors created by `seed`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub communities: usize,
    pub channels_per_community: usize,
    /// Top-level kind:9 messages seeded per channel.
    #[serde(default)]
    pub history_messages_per_channel: usize,
    /// Thread roots seeded per channel, each with `thread_depth` replies.
    #[serde(default)]
    pub thread_roots_per_channel: usize,
    #[serde(default)]
    pub thread_depth: usize,
    /// Concurrent seeding requests.
    #[serde(default = "default_seed_concurrency")]
    pub seed_concurrency: usize,
}

fn default_seed_concurrency() -> usize {
    32
}

/// Base (1x) population; ramp steps multiply it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Population {
    pub humans: usize,
    pub agents: usize,
    /// Channels each human and agent subscribes to and posts in.
    pub channels_per_member: usize,
    /// New connections per second per shard while ramping.
    #[serde(default = "default_connect_rate")]
    pub connect_rate_per_sec: f64,
}

fn default_connect_rate() -> f64 {
    200.0
}

/// Per-connection rates for a human client. Defaults are round placeholders;
/// set rates per scenario to model a specific deployment.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HumanProfile {
    pub presence_interval_secs: f64,
    pub typing_per_hour: f64,
    pub messages_per_hour: f64,
    pub reactions_per_hour: f64,
    /// Fraction of messages posted as replies into a seeded deep thread.
    pub thread_reply_fraction: f64,
    /// History events fetched per channel on connect (0 disables).
    pub history_limit: usize,
    /// Mean session length before a reconnect (0 keeps connections open).
    pub mean_session_secs: f64,
}

impl Default for HumanProfile {
    fn default() -> Self {
        Self {
            presence_interval_secs: 60.0,
            typing_per_hour: 10.0,
            messages_per_hour: 1.0,
            reactions_per_hour: 0.5,
            thread_reply_fraction: 0.3,
            history_limit: 50,
            mean_session_secs: 0.0,
        }
    }
}

/// Per-connection rates for an agent client.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentProfile {
    /// Mean interval between HTTP `/query` polls.
    pub query_interval_secs: f64,
    /// Messages posted through HTTP `/events` per hour.
    pub events_per_hour: f64,
    /// Encrypted kind:24200 observer frames sent to the owner per hour.
    pub observer_frames_per_hour: f64,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            query_interval_secs: 5.0,
            events_per_hour: 60.0,
            observer_frames_per_hour: 60.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RampStep {
    /// Multiplier on the base population.
    pub users: f64,
    pub hold_secs: u64,
}

/// Stop the ramp at the end of a step that crosses these limits.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Abort {
    pub max_error_rate: f64,
    pub max_p99_ms: f64,
}

impl Default for Abort {
    fn default() -> Self {
        Self {
            max_error_rate: 0.05,
            max_p99_ms: 5_000.0,
        }
    }
}

/// One community's endpoints.
#[derive(Clone, Debug)]
pub struct Community {
    pub index: usize,
    pub host: Option<String>,
    pub ws_url: String,
    pub http_url: String,
}

impl Scenario {
    pub fn parse(text: &str) -> Result<Self, String> {
        let scenario: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        scenario.validate()?;
        Ok(scenario)
    }

    fn validate(&self) -> Result<(), String> {
        if self.shape.communities == 0 || self.shape.channels_per_community == 0 {
            return Err("shape needs at least one community and one channel".into());
        }
        if self.shape.communities > 1 && self.target.community_host_template.is_none() {
            return Err("more than one community needs target.community_host_template".into());
        }
        if self.population.channels_per_member == 0
            || self.population.channels_per_member > self.shape.channels_per_community
        {
            return Err("population.channels_per_member must be 1..=channels_per_community".into());
        }
        if self.ramp.is_empty() || self.ramp.iter().any(|s| s.users <= 0.0) {
            return Err("ramp needs at least one step with users > 0".into());
        }
        if self.population.connect_rate_per_sec <= 0.0 {
            return Err("population.connect_rate_per_sec must be > 0".into());
        }
        Ok(())
    }

    pub fn communities(&self) -> Vec<Community> {
        (0..self.shape.communities)
            .map(|index| match &self.target.community_host_template {
                None => Community {
                    index,
                    host: None,
                    ws_url: self.target.ws_url.clone(),
                    http_url: self.target.http_url.clone(),
                },
                Some(template) => {
                    let host = template
                        .replace("{run}", &self.run_id)
                        .replace("{i}", &index.to_string());
                    Community {
                        index,
                        ws_url: with_host(&self.target.ws_url, &host),
                        http_url: with_host(&self.target.http_url, &host),
                        host: Some(host),
                    }
                }
            })
            .collect()
    }

    pub fn channel_ids(&self, community: usize) -> Vec<String> {
        (0..self.shape.channels_per_community)
            .map(|i| identity::channel_id(&self.run_id, community, i))
            .collect()
    }

    /// Channels member `member` (global human or agent index) belongs to.
    pub fn member_channels(&self, community: usize, member: usize) -> Vec<String> {
        let n = self.shape.channels_per_community;
        (0..self.population.channels_per_member)
            .map(|k| identity::channel_id(&self.run_id, community, (member + k) % n))
            .collect()
    }

    /// Largest multiplier any ramp step reaches.
    pub fn peak_users(&self) -> f64 {
        self.ramp.iter().map(|s| s.users).fold(0.0, f64::max)
    }
}

/// `url` with its host (and port) replaced by `host`.
fn with_host(url: &str, host: &str) -> String {
    match url.split_once("://") {
        Some((scheme, _)) => format!("{scheme}://{host}"),
        None => format!("{url}://{host}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
run_id = "t"
[target]
ws_url = "ws://localhost:3000"
http_url = "http://localhost:3000"
[shape]
communities = 1
channels_per_community = 4
[population]
humans = 10
agents = 2
channels_per_member = 2
[[ramp]]
users = 1.0
hold_secs = 10
"#;

    #[test]
    fn minimal_scenario_parses_with_profile_defaults() {
        let s = Scenario::parse(MINIMAL).unwrap();
        assert_eq!(s.communities().len(), 1);
        assert_eq!(s.communities()[0].ws_url, "ws://localhost:3000");
        assert_eq!(s.human.presence_interval_secs, 60.0);
        assert_eq!(s.member_channels(0, 3).len(), 2);
    }

    #[test]
    fn templated_hosts_keep_scheme_and_drop_port() {
        let text = MINIMAL
            .replace("communities = 1", "communities = 2")
            .replace(
                "http_url = \"http://localhost:3000\"",
                "http_url = \"http://localhost:3000\"\ncommunity_host_template = \"lg-{run}-{i}.example\"",
            );
        let s = Scenario::parse(&text).unwrap();
        let c = s.communities();
        assert_eq!(c[1].ws_url, "ws://lg-t-1.example");
        assert_eq!(c[1].http_url, "http://lg-t-1.example");
        assert_eq!(c[1].host.as_deref(), Some("lg-t-1.example"));
    }

    #[test]
    fn invalid_shapes_are_rejected() {
        assert!(Scenario::parse(&MINIMAL.replace("communities = 1", "communities = 2")).is_err());
        assert!(Scenario::parse(
            &MINIMAL.replace("channels_per_member = 2", "channels_per_member = 5")
        )
        .is_err());
        assert!(Scenario::parse(&MINIMAL.replace("users = 1.0", "users = 0.0")).is_err());
    }
}
