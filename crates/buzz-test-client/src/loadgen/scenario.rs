//! Scenario file schema and derived fixture layout.

use serde::Deserialize;

use super::identity::{self, Role};

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
    #[serde(default)]
    pub hot_thread: HotThread,
    #[serde(default)]
    pub desktop: DesktopProfile,
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
    /// Dial community hosts here (`host:port`) in plaintext instead of
    /// resolving them, e.g. the relay's in-cluster service. `ws_url` and
    /// `http_url` keep the community's public scheme: Host headers and signed
    /// NIP-98 and NIP-42 URLs still name the community at that scheme.
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
    /// Create channels private and add every member with kind:9000, so the
    /// relay checks real membership on every read and subscription. Off
    /// creates open channels nobody joins.
    #[serde(default)]
    pub private_channels: bool,
    /// The first `wide_channels` channels of each community are its wide
    /// channels, which `population.wide_member_fraction` of humans join on
    /// top of their own. Other memberships use the remaining channels.
    #[serde(default)]
    pub wide_channels: usize,
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
    /// Fraction of humans that also join every wide channel.
    #[serde(default)]
    pub wide_member_fraction: f64,
    /// Share of its typing, messages and reactions a member of the wide
    /// channels sends into them; the rest goes to its regular channels.
    /// Unset, typing and messages pick any of a member's channels equally and
    /// reactions go to the latest message it saw.
    #[serde(default)]
    pub wide_activity_share: Option<f64>,
    /// Humans whose channel count differs from `channels_per_member`, such
    /// as a few heavy users in many channels. Agents keep `channels_per_member`.
    #[serde(default)]
    pub tiers: Vec<Tier>,
}

/// A member's split of its own traffic between the wide channels that lead
/// its channel list and its regular channels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WideSplit {
    /// Wide channels at the start of the member's channels.
    pub wide: usize,
    /// Share of picks that land in a wide channel.
    pub share: f64,
}

/// `fraction` of humans join `channels` channels instead of
/// `population.channels_per_member`, not counting wide channels.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    pub fraction: f64,
    pub channels: usize,
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
    /// Mean interval between HTTP `/query` polls (0 disables polling).
    pub query_interval_secs: f64,
    /// Messages posted through HTTP `/events` per hour.
    pub events_per_hour: f64,
    /// Encrypted kind:24200 observer frames sent to the owner per hour,
    /// outside turns.
    pub observer_frames_per_hour: f64,
    /// Seconds between presence heartbeats (0 sends none).
    pub presence_interval_secs: f64,
    /// Open one live REQ per channel for the agent's mentions there, as the
    /// agent harness does, instead of relying on HTTP polling.
    pub channel_subscriptions: bool,
    pub turn: AgentTurn,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            query_interval_secs: 5.0,
            events_per_hour: 60.0,
            observer_frames_per_hour: 60.0,
            presence_interval_secs: 0.0,
            channel_subscriptions: false,
            turn: AgentTurn::default(),
        }
    }
}

/// Agent turns: bursts of work in one channel, as an agent harness runs a
/// prompt. A turn starts with context reads and two reactions, streams
/// typing and observer frames while it runs, makes `reads` tool reads, and
/// ends with a reply and the reactions' removal. Disabled at 0 `per_hour`.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentTurn {
    /// Turns started per hour while the agent is idle. An agent runs one
    /// turn at a time.
    pub per_hour: f64,
    /// Mean turn length. Each turn lasts 0.5–1.5 times this.
    pub mean_secs: f64,
    /// Seconds between observer frames during a turn (0 sends none).
    pub frame_secs: f64,
    /// Plaintext size of each observer frame, before encryption.
    pub frame_bytes: usize,
    /// Seconds between typing indicators during a turn (0 sends none).
    pub typing_secs: f64,
    /// Channel reads over HTTP `/query` during a turn, as tool calls.
    pub reads: usize,
}

impl Default for AgentTurn {
    fn default() -> Self {
        Self {
            per_hour: 0.0,
            mean_secs: 60.0,
            frame_secs: 1.0,
            frame_bytes: 1024,
            typing_secs: 3.0,
            reads: 2,
        }
    }
}

/// Largest observer frame plaintext: NIP-44 encrypts at most 65,535 bytes,
/// and the frame's JSON envelope needs some of them.
pub const MAX_FRAME_BYTES: usize = 60_000;

/// One thread in community 0, channel 0 that keeps growing: seeded with
/// many direct replies, then replied to and read throughout the run.
/// Disabled while `seed_replies` and `authors` are both 0.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HotThread {
    /// Direct replies seeded under the hot root.
    pub seed_replies: usize,
    /// Clients replying to the hot root at 1x; ramp steps multiply them.
    pub authors: usize,
    /// Replies each author posts per hour through HTTP `/events`.
    pub replies_per_hour: f64,
    /// Thread reads (the root's newest 100 replies) each author makes per
    /// hour through HTTP `/query`.
    pub fetches_per_hour: f64,
}

impl HotThread {
    pub fn enabled(&self) -> bool {
        self.seed_replies > 0 || self.authors > 0
    }
}

/// Which desktop app request pattern a desktop client follows.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DesktopMode {
    /// Before the client volume fixes: every recovery tick replaces each live
    /// channel batch and rereads the roster, read state and unreads, and an
    /// identity renewal is a full reconnect.
    Legacy,
    /// After them: tick reads and batch re-sends wait for a backstop, and a
    /// renewal re-sends subscriptions from their cursors.
    #[default]
    Current,
}

/// Humans that run the desktop app's relay request pattern instead of the
/// basic human client: HTTP reads of the roster, profiles, read state,
/// unreads and open windows on connect, batched channel subscriptions, a
/// recovery tick, and optional periodic identity renewal. Disabled while
/// `fraction` is 0.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopProfile {
    /// Fraction of humans that are desktop clients.
    pub fraction: f64,
    pub mode: DesktopMode,
    /// Identity renewal interval, a reconnect the client plans (0: never).
    pub renewal_secs: f64,
    /// Channel windows kept open, each caught up on every establishment.
    pub windows: usize,
    /// Thread views kept open, each refreshed on every establishment.
    pub threads: usize,
    /// Recovery tick interval.
    pub tick_secs: f64,
    /// `current` mode's backstop for tick reads and batch re-sends.
    pub backstop_secs: f64,
}

impl Default for DesktopProfile {
    fn default() -> Self {
        Self {
            fraction: 0.0,
            mode: DesktopMode::Current,
            renewal_secs: 0.0,
            windows: 3,
            threads: 1,
            tick_secs: 60.0,
            backstop_secs: 900.0,
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

/// A member of channels: a human or agent by global index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub role: Role,
    pub index: usize,
}

/// Members of every channel, indexed `[community][channel]`.
pub type Members = Vec<Vec<Vec<Member>>>;

/// Clients each kind reaches at the largest ramp step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peak {
    pub humans: usize,
    pub agents: usize,
    pub hot_authors: usize,
}

/// Salts for [`draw`], one per independent per-member choice.
const TIER_SALT: u64 = 1;
const WIDE_SALT: u64 = 2;
const DESKTOP_SALT: u64 = 3;

/// A deterministic draw in `[0, 1)` for member `index`, independent for each
/// `salt` and of the client schedules, so every shard and `seed` agree.
fn draw(index: usize, salt: u64) -> f64 {
    let mut z = (index as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xD1B5_4A32_D192_ED03));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
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
        let n = self.shape.channels_per_community;
        let wide = self.shape.wide_channels;
        // Every member needs at least one channel outside the wide ones.
        if wide >= n && wide > 0 {
            return Err("shape.wide_channels must be below channels_per_community".into());
        }
        if self.population.channels_per_member == 0
            || self.population.channels_per_member > n - wide
        {
            return Err(
                "population.channels_per_member must be 1..=channels_per_community - wide_channels"
                    .into(),
            );
        }
        let mut tiered = 0.0;
        for tier in &self.population.tiers {
            if !(tier.fraction > 0.0 && tier.fraction <= 1.0) {
                return Err("population.tiers fraction must be above 0 and at most 1".into());
            }
            if tier.channels == 0 || tier.channels > n - wide {
                return Err(
                    "population.tiers channels must be 1..=channels_per_community - wide_channels"
                        .into(),
                );
            }
            tiered += tier.fraction;
        }
        if tiered > 1.0 + 1e-9 {
            return Err("population.tiers fractions must sum to at most 1".into());
        }
        if self.ramp.is_empty() {
            return Err("ramp needs at least one step".into());
        }
        // Clients are only ever added, so a step can never shrink the
        // population. Repeating a level splits a long hold into reported,
        // judged windows.
        let mut previous = 0.0;
        for step in &self.ramp {
            if !step.users.is_finite() || step.users <= 0.0 || step.users < previous {
                return Err("ramp users must be finite, above 0 and never decrease".into());
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
            ("agent.turn.mean_secs", a.turn.mean_secs),
            ("abort.max_p99_ms", self.abort.max_p99_ms),
            ("desktop.backstop_secs", self.desktop.backstop_secs),
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
        // Neither the recovery tick nor a renewal runs more than once a second.
        let d = &self.desktop;
        if !d.tick_secs.is_finite() || d.tick_secs < 1.0 {
            return Err("desktop.tick_secs must be a finite number >= 1".into());
        }
        if !d.renewal_secs.is_finite() || !(d.renewal_secs == 0.0 || d.renewal_secs >= 1.0) {
            return Err("desktop.renewal_secs must be 0 or a finite number >= 1".into());
        }
        // Agent intervals are 0 (off) or slow enough to schedule.
        let intervals = [
            (
                "agent.presence_interval_secs",
                a.presence_interval_secs,
                1.0,
            ),
            ("agent.turn.frame_secs", a.turn.frame_secs, 0.1),
            ("agent.turn.typing_secs", a.turn.typing_secs, 0.1),
        ];
        for (name, value, min) in intervals {
            if !value.is_finite() || !(value == 0.0 || value >= min) {
                return Err(format!("{name} must be 0 or a finite number >= {min}"));
            }
        }
        if a.turn.frame_bytes > MAX_FRAME_BYTES {
            return Err(format!(
                "agent.turn.frame_bytes must be at most {MAX_FRAME_BYTES}"
            ));
        }
        let non_negative = [
            ("human.typing_per_hour", h.typing_per_hour),
            ("human.messages_per_hour", h.messages_per_hour),
            ("human.reactions_per_hour", h.reactions_per_hour),
            ("human.mean_session_secs", h.mean_session_secs),
            ("agent.events_per_hour", a.events_per_hour),
            ("agent.observer_frames_per_hour", a.observer_frames_per_hour),
            ("agent.query_interval_secs", a.query_interval_secs),
            ("agent.turn.per_hour", a.turn.per_hour),
            (
                "hot_thread.replies_per_hour",
                self.hot_thread.replies_per_hour,
            ),
            (
                "hot_thread.fetches_per_hour",
                self.hot_thread.fetches_per_hour,
            ),
        ];
        for (name, value) in non_negative {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be a finite number >= 0"));
            }
        }
        let fractions = [
            ("human.thread_reply_fraction", h.thread_reply_fraction),
            ("abort.max_error_rate", self.abort.max_error_rate),
            (
                "population.wide_member_fraction",
                self.population.wide_member_fraction,
            ),
            ("desktop.fraction", self.desktop.fraction),
            (
                "population.wide_activity_share",
                self.population.wide_activity_share.unwrap_or(0.0),
            ),
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

    /// Indices, within its community, of the channels member `index` of
    /// `role` (a global human or agent index) belongs to: the wide channels
    /// if it joins them, then a run of consecutive regular channels starting
    /// at its own index. Without wide channels or tiers this is
    /// `channels_per_member` channels from `index % channels_per_community`.
    pub fn member_channel_indices(&self, role: Role, index: usize) -> Vec<usize> {
        let wide = self.shape.wide_channels;
        let regular = self.shape.channels_per_community - wide;
        let mut channels = Vec::new();
        let mut count = self.population.channels_per_member;
        if self.joins_wide(role, index) {
            channels.extend(0..wide);
        }
        if role == Role::Human {
            count = self.tier_channels(index);
        }
        channels.extend((0..count).map(|k| wide + (index + k) % regular));
        channels
    }

    /// Whether member `index` of `role` joins the wide channels.
    fn joins_wide(&self, role: Role, index: usize) -> bool {
        role == Role::Human
            && self.shape.wide_channels > 0
            && draw(index, WIDE_SALT) < self.population.wide_member_fraction
    }

    /// How member `index` of `role` splits its own traffic, when
    /// `population.wide_activity_share` applies to it: the number of wide
    /// channels leading its channel list and the share they get.
    pub fn wide_split(&self, role: Role, index: usize) -> Option<WideSplit> {
        let share = self.population.wide_activity_share?;
        self.joins_wide(role, index).then_some(WideSplit {
            wide: self.shape.wide_channels,
            share,
        })
    }

    /// Regular (not wide) channels human `index` joins: its tier's count, or
    /// `channels_per_member` for humans outside every tier.
    fn tier_channels(&self, index: usize) -> usize {
        let u = draw(index, TIER_SALT);
        let mut bound = 0.0;
        for tier in &self.population.tiers {
            bound += tier.fraction;
            if u < bound {
                return tier.channels;
            }
        }
        self.population.channels_per_member
    }

    /// Channel ids member `index` of `role` belongs to in `community`.
    pub fn member_channels(&self, community: usize, role: Role, index: usize) -> Vec<String> {
        self.member_channel_indices(role, index)
            .into_iter()
            .map(|channel| identity::channel_id(&self.run_id, community, channel))
            .collect()
    }

    /// Whether human `index` runs the desktop client.
    pub fn is_desktop(&self, index: usize) -> bool {
        draw(index, DESKTOP_SALT) < self.desktop.fraction
    }

    /// Largest multiplier any ramp step reaches.
    pub fn peak_users(&self) -> f64 {
        self.ramp.iter().map(|s| s.users).fold(0.0, f64::max)
    }

    /// Clients of each kind at the largest ramp step, rounded as `run` does.
    pub fn peak(&self) -> Peak {
        let at = |base: usize| (base as f64 * self.peak_users()).ceil() as usize;
        Peak {
            humans: at(self.population.humans),
            agents: at(self.population.agents),
            hot_authors: at(self.hot_thread.authors),
        }
    }

    /// Humans and agents of every channel up to the peak population, indexed
    /// `[community][channel]`, humans first and each kind by index. A client
    /// with global index `g` lives in community `g % communities`.
    pub fn channel_members(&self) -> Members {
        let shape = &self.shape;
        let mut members = vec![vec![Vec::new(); shape.channels_per_community]; shape.communities];
        let peak = self.peak();
        for (role, count) in [(Role::Human, peak.humans), (Role::Agent, peak.agents)] {
            for index in 0..count {
                let community = &mut members[index % shape.communities];
                for channel in self.member_channel_indices(role, index) {
                    community[channel].push(Member { role, index });
                }
            }
        }
        members
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
        assert_eq!(s.member_channels(0, Role::Human, 3).len(), 2);
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
            "[agent]\nquery_interval_secs = -1.0\n",
            "[agent]\npresence_interval_secs = 0.5\n",
            "[agent.turn]\nper_hour = -1.0\n",
            "[agent.turn]\nmean_secs = 0.0\n",
            "[agent.turn]\nframe_secs = 0.05\n",
            "[agent.turn]\ntyping_secs = nan\n",
            "[agent.turn]\nframe_bytes = 60001\n",
            "[abort]\nmax_error_rate = nan\n",
            "[abort]\nmax_p99_ms = inf\n",
            "[[ramp]]\nusers = inf\nhold_secs = 10\n",
            // A down-ramp would report a smaller population than is running.
            "[[ramp]]\nusers = 0.5\nhold_secs = 10\n",
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
        // A repeated level splits a long hold into judged windows.
        let held = format!("{MINIMAL}[[ramp]]\nusers = 1.0\nhold_secs = 10\n");
        assert!(Scenario::parse(&held).is_ok());
        // Polling, presence, frames and typing can each be switched off.
        let off = format!(
            "{MINIMAL}[agent]\nquery_interval_secs = 0.0\npresence_interval_secs = 0.0\n\
             [agent.turn]\nper_hour = 6.0\nframe_secs = 0.0\ntyping_secs = 0.0\n"
        );
        assert!(Scenario::parse(&off).is_ok());
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

    /// MINIMAL with `n` channels, `shape` and `population` lines added, and
    /// `extra` TOML after it.
    fn shaped(n: usize, shape: &str, population: &str, extra: &str) -> Result<Scenario, String> {
        let text = MINIMAL
            .replace(
                "channels_per_community = 4",
                &format!("channels_per_community = {n}\n{shape}"),
            )
            .replace(
                "channels_per_member = 2",
                &format!("channels_per_member = 2\n{population}"),
            );
        Scenario::parse(&format!("{text}{extra}"))
    }

    #[test]
    fn without_wide_channels_or_tiers_every_member_keeps_its_run_of_channels() {
        let s = Scenario::parse(MINIMAL).unwrap();
        for role in [Role::Human, Role::Agent] {
            for m in 0..50 {
                let want: Vec<usize> = (0..2).map(|k| (m + k) % 4).collect();
                assert_eq!(s.member_channel_indices(role, m), want, "{role:?} {m}");
            }
        }
        let ids = s.member_channels(0, Role::Human, 3);
        assert_eq!(ids, [3, 0].map(|c| identity::channel_id("t", 0, c)));
        assert_eq!(s.desktop.fraction, 0.0);
        assert!(!(0..1000).any(|m| s.is_desktop(m)));
        assert!(!s.shape.private_channels);
    }

    #[test]
    fn wide_channels_and_tiers_are_deterministic_and_honor_their_fractions() {
        let s = shaped(
            12,
            "wide_channels = 2",
            "wide_member_fraction = 0.5\n[[population.tiers]]\nfraction = 0.1\nchannels = 8\n[[population.tiers]]\nfraction = 0.2\nchannels = 1",
            "[desktop]\nfraction = 0.25\n",
        )
        .unwrap();
        let again = shaped(
            12,
            "wide_channels = 2",
            "wide_member_fraction = 0.5\n[[population.tiers]]\nfraction = 0.1\nchannels = 8\n[[population.tiers]]\nfraction = 0.2\nchannels = 1",
            "[desktop]\nfraction = 0.25\n",
        )
        .unwrap();
        let n = 20_000;
        let (mut wide, mut heavy, mut light, mut desktop) = (0, 0, 0, 0);
        for m in 0..n {
            let channels = s.member_channel_indices(Role::Human, m);
            assert_eq!(channels, again.member_channel_indices(Role::Human, m));
            let joins_wide = channels.starts_with(&[0, 1]);
            let regular: Vec<usize> = channels.iter().copied().filter(|c| *c >= 2).collect();
            assert_eq!(regular.len() + 2 * usize::from(joins_wide), channels.len());
            // Regular channels are a run of distinct channels from the member's index.
            let want: Vec<usize> = (0..regular.len()).map(|k| 2 + (m + k) % 10).collect();
            assert_eq!(regular, want, "member {m}");
            wide += usize::from(joins_wide);
            heavy += usize::from(regular.len() == 8);
            light += usize::from(regular.len() == 1);
            desktop += usize::from(s.is_desktop(m));
            // Agents never join wide channels and keep channels_per_member.
            let agent = s.member_channel_indices(Role::Agent, m);
            assert_eq!(agent, [2 + m % 10, 2 + (m + 1) % 10]);
        }
        let share = |count: usize| count as f64 / n as f64;
        assert!((share(wide) - 0.5).abs() < 0.02, "wide {wide}");
        assert!((share(heavy) - 0.1).abs() < 0.02, "heavy {heavy}");
        assert!((share(light) - 0.2).abs() < 0.02, "light {light}");
        assert!((share(desktop) - 0.25).abs() < 0.02, "desktop {desktop}");
    }

    #[test]
    fn channel_members_inverts_the_assignment_up_to_the_peak() {
        let s = shaped(
            6,
            "wide_channels = 1",
            "wide_member_fraction = 0.3\n[[population.tiers]]\nfraction = 0.5\nchannels = 4",
            "[[ramp]]\nusers = 2.5\nhold_secs = 10\n",
        )
        .unwrap();
        let peak = s.peak();
        assert_eq!((peak.humans, peak.agents), (25, 5));
        let members = s.channel_members();
        assert_eq!(members.len(), 1);
        let mut listed = 0;
        for (role, count) in [(Role::Human, 25), (Role::Agent, 5)] {
            for index in 0..count {
                for c in s.member_channel_indices(role, index) {
                    assert!(members[0][c].contains(&Member { role, index }));
                    listed += 1;
                }
            }
        }
        assert_eq!(listed, members[0].iter().map(Vec::len).sum::<usize>());
        // Humans come first in every channel.
        for channel in &members[0] {
            let first_agent = channel.iter().position(|m| m.role == Role::Agent);
            let last_human = channel.iter().rposition(|m| m.role == Role::Human);
            if let (Some(a), Some(h)) = (first_agent, last_human) {
                assert!(h < a);
            }
        }
    }

    #[test]
    fn wide_activity_share_applies_only_to_members_of_wide_channels() {
        let wide = "wide_channels = 2";
        let unset = shaped(6, wide, "wide_member_fraction = 1.0", "").unwrap();
        assert_eq!(unset.population.wide_activity_share, None);
        assert_eq!(unset.wide_split(Role::Human, 0), None);
        let set = shaped(
            6,
            wide,
            "wide_member_fraction = 1.0\nwide_activity_share = 0.25",
            "",
        )
        .unwrap();
        let split = WideSplit {
            wide: 2,
            share: 0.25,
        };
        for m in 0..20 {
            assert_eq!(set.wide_split(Role::Human, m), Some(split));
            // The wide channels lead the member's channels, as the split assumes.
            assert_eq!(set.member_channel_indices(Role::Human, m)[..2], [0, 1]);
            // Agents never join wide channels.
            assert_eq!(set.wide_split(Role::Agent, m), None);
        }
        let outside = shaped(6, wide, "wide_activity_share = 0.25", "").unwrap();
        assert!((0..20).all(|m| outside.wide_split(Role::Human, m).is_none()));
    }

    #[test]
    fn shape_and_desktop_knobs_are_validated() {
        let rejected = [
            ("wide_channels = 4", "", ""),
            // Two members' channels no longer fit outside three wide channels.
            ("wide_channels = 3", "", ""),
            ("", "wide_member_fraction = 1.5", ""),
            ("", "wide_member_fraction = nan", ""),
            ("", "wide_activity_share = 1.5", ""),
            ("", "wide_activity_share = -0.1", ""),
            ("", "wide_activity_share = nan", ""),
            ("", "[[population.tiers]]\nfraction = 0.0\nchannels = 1", ""),
            ("", "[[population.tiers]]\nfraction = 1.5\nchannels = 1", ""),
            ("", "[[population.tiers]]\nfraction = 0.6\nchannels = 1\n[[population.tiers]]\nfraction = 0.6\nchannels = 2", ""),
            ("", "[[population.tiers]]\nfraction = 0.5\nchannels = 0", ""),
            ("", "[[population.tiers]]\nfraction = 0.5\nchannels = 5", ""),
            ("wide_channels = 1", "[[population.tiers]]\nfraction = 0.5\nchannels = 4", ""),
            ("", "[[population.tiers]]\nfraction = 0.5\nchannels = 1\nextra = 1", ""),
            ("", "", "[desktop]\nfraction = 1.5\n"),
            ("", "", "[desktop]\nfraction = -0.1\n"),
            ("", "", "[desktop]\nmode = \"bogus\"\n"),
            ("", "", "[desktop]\ntick_secs = 0.5\n"),
            ("", "", "[desktop]\ntick_secs = inf\n"),
            ("", "", "[desktop]\nrenewal_secs = 0.5\n"),
            ("", "", "[desktop]\nrenewal_secs = -1.0\n"),
            ("", "", "[desktop]\nbackstop_secs = 0.0\n"),
            ("", "", "[desktop]\nwindow = 1\n"),
        ];
        for (shape, population, extra) in rejected {
            assert!(
                shaped(4, shape, population, extra).is_err(),
                "accepted: {shape} {population} {extra}"
            );
        }
        let accepted = [
            ("wide_channels = 2", "wide_member_fraction = 1.0", ""),
            ("private_channels = true", "", ""),
            (
                "",
                "[[population.tiers]]\nfraction = 0.5\nchannels = 4\n[[population.tiers]]\nfraction = 0.5\nchannels = 1",
                "",
            ),
            (
                "",
                "",
                "[desktop]\nfraction = 1.0\nmode = \"legacy\"\nrenewal_secs = 1.0\ntick_secs = 1.0\nwindows = 0\nthreads = 0\n",
            ),
        ];
        for (shape, population, extra) in accepted {
            let s = shaped(4, shape, population, extra)
                .unwrap_or_else(|e| panic!("rejected {shape} {population} {extra}: {e}"));
            assert!(!s.member_channel_indices(Role::Human, 0).is_empty());
        }
        let legacy = shaped(4, "", "", "[desktop]\nmode = \"legacy\"\n").unwrap();
        assert_eq!(legacy.desktop.mode, DesktopMode::Legacy);
        assert_eq!(
            Scenario::parse(MINIMAL).unwrap().desktop.mode,
            DesktopMode::Current
        );
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
