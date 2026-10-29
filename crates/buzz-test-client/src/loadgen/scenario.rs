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
    /// Deadline for one WebSocket connect plus NIP-42 authentication.
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: f64,
}

fn default_operator_key_env() -> String {
    "LOADGEN_OPERATOR_KEY".into()
}

fn default_connect_timeout() -> f64 {
    10.0
}

/// Deepest reply nesting the relay accepts.
const MAX_THREAD_DEPTH: usize = 100;

/// Data-shape vectors created by `seed`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub communities: usize,
    pub channels_per_community: usize,
    /// Top-level kind:9 messages seeded per channel.
    #[serde(default)]
    pub history_messages_per_channel: usize,
    /// Thread roots seeded per channel, each a chain of `thread_depth` nested
    /// replies (at most 100, the relay's limit).
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
        // `buffer_unordered(0)` never polls its input, so seeding would hang.
        if self.shape.seed_concurrency == 0 {
            return Err("shape.seed_concurrency must be at least 1".into());
        }
        if self.shape.communities > 1 && self.target.community_host_template.is_none() {
            return Err("more than one community needs target.community_host_template".into());
        }
        // The relay rejects replies nested deeper than this.
        if self.shape.thread_depth > MAX_THREAD_DEPTH {
            return Err(format!(
                "shape.thread_depth must be at most {MAX_THREAD_DEPTH}"
            ));
        }
        // The HTTP override resolves a bare hostname, so a host with a port
        // would reach `connect_addr` over WebSocket but not over HTTP.
        if self.target.connect_addr.is_some()
            && self
                .target
                .community_host_template
                .as_deref()
                .is_some_and(|template| template.contains(':'))
        {
            return Err(
                "target.community_host_template must not include a port when target.connect_addr is set"
                    .into(),
            );
        }
        if self.population.channels_per_member == 0
            || self.population.channels_per_member > self.shape.channels_per_community
        {
            return Err("population.channels_per_member must be 1..=channels_per_community".into());
        }
        if self.ramp.is_empty() {
            return Err("ramp needs at least one step".into());
        }
        // Clients are only ever added, so a step can never shrink the population.
        let mut previous = 0.0;
        for step in &self.ramp {
            if !step.users.is_finite() || step.users <= previous {
                return Err("ramp users must be finite and increase every step".into());
            }
            previous = step.users;
        }
        let (h, a) = (&self.human, &self.agent);
        let positive = [
            (
                "target.connect_timeout_secs",
                self.target.connect_timeout_secs,
            ),
            (
                "population.connect_rate_per_sec",
                self.population.connect_rate_per_sec,
            ),
            ("agent.query_interval_secs", a.query_interval_secs),
            ("abort.max_p99_ms", self.abort.max_p99_ms),
        ];
        for (name, value) in positive {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("{name} must be a finite number > 0"));
            }
        }
        // Presence heartbeats are never sent more often than once a second.
        if !h.presence_interval_secs.is_finite() || h.presence_interval_secs < 1.0 {
            return Err("human.presence_interval_secs must be a finite number >= 1".into());
        }
        let non_negative = [
            ("human.typing_per_hour", h.typing_per_hour),
            ("human.messages_per_hour", h.messages_per_hour),
            ("human.reactions_per_hour", h.reactions_per_hour),
            ("human.mean_session_secs", h.mean_session_secs),
            ("agent.events_per_hour", a.events_per_hour),
            ("agent.observer_frames_per_hour", a.observer_frames_per_hour),
        ];
        for (name, value) in non_negative {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be a finite number >= 0"));
            }
        }
        let fractions = [
            ("human.thread_reply_fraction", h.thread_reply_fraction),
            ("abort.max_error_rate", self.abort.max_error_rate),
        ];
        for (name, value) in fractions {
            if !(0.0..=1.0).contains(&value) {
                return Err(format!("{name} must be between 0 and 1"));
            }
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

    #[test]
    fn unschedulable_values_are_rejected() {
        let cases = [
            "[human]\nmessages_per_hour = nan\n",
            "[human]\ntyping_per_hour = inf\n",
            "[human]\nreactions_per_hour = -1.0\n",
            "[human]\npresence_interval_secs = 0.0\n",
            "[human]\npresence_interval_secs = 0.5\n",
            "[human]\nthread_reply_fraction = 1.5\n",
            "[agent]\nquery_interval_secs = nan\n",
            "[agent]\nobserver_frames_per_hour = inf\n",
            "[abort]\nmax_error_rate = nan\n",
            "[abort]\nmax_p99_ms = inf\n",
            "[[ramp]]\nusers = inf\nhold_secs = 10\n",
            // A down-ramp would report a smaller population than is running.
            "[[ramp]]\nusers = 0.5\nhold_secs = 10\n",
            "[[ramp]]\nusers = 1.0\nhold_secs = 10\n",
        ];
        for case in cases {
            let text = format!("{MINIMAL}{case}");
            assert!(Scenario::parse(&text).is_err(), "accepted: {case}");
        }
        let text = MINIMAL.replace(
            "http_url = \"http://localhost:3000\"",
            "http_url = \"http://localhost:3000\"\nconnect_timeout_secs = nan",
        );
        assert!(Scenario::parse(&text).is_err());
        let up = format!("{MINIMAL}[[ramp]]\nusers = 2.0\nhold_secs = 10\n");
        assert!(Scenario::parse(&up).is_ok());
    }

    #[test]
    fn shipped_scenarios_validate() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../perf/scenarios");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                let text = std::fs::read_to_string(&path).unwrap();
                Scenario::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn presence_interval_of_one_second_is_the_minimum() {
        let with = |secs: &str| format!("{MINIMAL}[human]\npresence_interval_secs = {secs}\n");
        assert!(Scenario::parse(&with("1.0")).is_ok());
        assert!(Scenario::parse(&with("0.99")).is_err());
    }

    #[test]
    fn zero_seed_concurrency_is_rejected() {
        let with = |n: usize| {
            MINIMAL.replace(
                "channels_per_community = 4",
                &format!("channels_per_community = 4\nseed_concurrency = {n}"),
            )
        };
        assert!(Scenario::parse(&with(1)).is_ok());
        let err = Scenario::parse(&with(0)).unwrap_err();
        assert!(err.contains("seed_concurrency"), "{err}");
    }

    #[test]
    fn host_template_ports_conflict_with_connect_addr() {
        let with = |template: &str, connect_addr: Option<&str>| {
            let connect = connect_addr
                .map(|a| format!("\nconnect_addr = \"{a}\""))
                .unwrap_or_default();
            MINIMAL.replace(
                "http_url = \"http://localhost:3000\"",
                &format!(
                    "http_url = \"http://localhost:3000\"\ncommunity_host_template = \"{template}\"{connect}"
                ),
            )
        };
        // A port is fine while hosts resolve normally, and so is an override
        // for a portless host.
        assert!(Scenario::parse(&with("lg-{run}-{i}.example:8443", None)).is_ok());
        assert!(Scenario::parse(&with("lg-{run}-{i}.example", Some("10.0.0.1:8080"))).is_ok());
        let err =
            Scenario::parse(&with("lg-{run}-{i}.example:8443", Some("10.0.0.1:8080"))).unwrap_err();
        assert!(err.contains("port"), "{err}");
    }

    #[test]
    fn thread_depth_is_limited_to_what_the_relay_accepts() {
        let with_depth = |depth: usize| {
            MINIMAL.replace(
                "channels_per_community = 4",
                &format!("channels_per_community = 4\nthread_depth = {depth}"),
            )
        };
        assert!(Scenario::parse(&with_depth(100)).is_ok());
        let err = Scenario::parse(&with_depth(101)).unwrap_err();
        assert!(err.contains("thread_depth"), "{err}");
    }
}
