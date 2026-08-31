//! Registry seam for sources that contribute to an agent's `team_instructions`
//! layer (rendered by both the modern `with_team` system-prompt path and the
//! legacy `format_prompt` `[Team Instructions]` section — one value, both
//! paths).
//!
//! As Buzz leans further into agents as first-class participants, more
//! sources will legitimately want to contribute standing context: workspace
//! house rules, per-team norms, channel-scoped policy, agent memory, and
//! whatever future capabilities add. Rather than hardcoding another
//! fetch-and-fold at the startup call site for each one, sources register as
//! an ordered [`TeamContextProvider`] and [`build_team_instructions`] folds
//! them together with the base (persona/team-supplied) `team_instructions`.
//!
//! No built-in provider is registered yet — [`builtin_team_context_providers`]
//! starts empty, so today's fold is the identity: the base instructions pass
//! through unchanged, exactly as `tokio_main` did before this seam existed.
//! This is the extension point future context sources register through.

use std::future::Future;
use std::pin::Pin;

/// Context handed to each [`TeamContextProvider`] so it can fetch its
/// contribution. Fields are borrowed references — providers run once at
/// startup, before any of these values are mutated or moved.
///
/// Unread today (`builtin_team_context_providers` is empty, so no provider
/// exists yet to read them) — kept `pub` and unsuppressed via `dead_code`
/// so the first real provider's use of them shows up as ordinary code, not
/// a lint fix.
#[allow(dead_code)]
pub struct TeamContextCtx<'a> {
    /// Relay URL to fetch from (for providers that talk to the relay).
    pub relay_url: &'a str,
    /// Agent's Nostr keys, used to authenticate relay connections.
    pub keys: &'a nostr::Keys,
}

/// A source of team-level context folded into `team_instructions` at agent
/// startup. Each provider returns its own already-framed contribution (or
/// `None` if it has nothing to add); [`build_team_instructions`] runs the
/// registered providers in order and joins their outputs ahead of the
/// persona/team-supplied base instructions.
///
/// Object-safe via a boxed future (this workspace does not depend on
/// `async-trait`) — mirrors the pattern used by `buzz_workflow::ActionSink`.
pub trait TeamContextProvider: Send + Sync {
    /// Stable identifier used for ordering/diagnostics (e.g. logging).
    fn name(&self) -> &'static str;

    /// Fetch and frame this provider's contribution to `team_instructions`.
    /// Returns `None` when the provider has nothing to add.
    fn provide<'a>(
        &'a self,
        ctx: &'a TeamContextCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>>;
}

/// The built-in, ordered list of [`TeamContextProvider`]s run at agent
/// startup. Empty today — no context source has been wired through this
/// seam yet. Persona/team base instructions are not a provider — they are
/// the `base` argument to [`build_team_instructions`], since they come from
/// config rather than a fetch.
pub fn builtin_team_context_providers() -> Vec<Box<dyn TeamContextProvider>> {
    Vec::new()
}

/// Run each provider in order, collect the framed contributions that
/// returned `Some`, then join `[provider outputs…, base]` (each trimmed,
/// empties dropped) with a blank line between sections. Returns `None` when
/// there is nothing to inject.
///
/// With today's empty provider list, this is the identity: it reproduces
/// `base` (trimmed, or `None` if blank) exactly.
pub async fn build_team_instructions(
    providers: &[Box<dyn TeamContextProvider>],
    ctx: &TeamContextCtx<'_>,
    base: Option<&str>,
) -> Option<String> {
    let mut sections: Vec<String> = Vec::with_capacity(providers.len() + 1);
    for provider in providers {
        match provider.provide(ctx).await {
            Some(contribution) => {
                let trimmed = contribution.trim();
                if !trimmed.is_empty() {
                    sections.push(trimmed.to_string());
                }
            }
            None => tracing::debug!(
                provider = provider.name(),
                "team context provider yielded nothing"
            ),
        }
    }
    if let Some(base) = base.map(str::trim).filter(|b| !b.is_empty()) {
        sections.push(base.to_string());
    }
    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(keys: &nostr::Keys) -> TeamContextCtx<'_> {
        TeamContextCtx {
            relay_url: "wss://example.invalid",
            keys,
        }
    }

    /// With no providers registered (today's built-in list), the fold must
    /// be the identity on `base` — this is the property that makes adding
    /// the seam behavior-preserving relative to the direct
    /// `config.team_instructions.clone()` pass-through it replaces.
    #[tokio::test]
    async fn empty_providers_is_identity_on_base() {
        let keys = nostr::Keys::generate();
        let c = ctx(&keys);
        let providers: Vec<Box<dyn TeamContextProvider>> = Vec::new();

        assert_eq!(
            build_team_instructions(&providers, &c, Some("Be terse.")).await,
            Some("Be terse.".to_string())
        );
        assert_eq!(build_team_instructions(&providers, &c, None).await, None);
        assert_eq!(
            build_team_instructions(&providers, &c, Some("   ")).await,
            None
        );
    }

    /// `builtin_team_context_providers` starts empty — no context source is
    /// wired through this seam yet.
    #[test]
    fn builtin_providers_start_empty() {
        assert!(builtin_team_context_providers().is_empty());
    }

    struct StubProvider(&'static str, Option<&'static str>);

    impl TeamContextProvider for StubProvider {
        fn name(&self) -> &'static str {
            self.0
        }

        fn provide<'a>(
            &'a self,
            _ctx: &'a TeamContextCtx<'a>,
        ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>> {
            let out = self.1.map(str::to_string);
            Box::pin(async move { out })
        }
    }

    /// Providers run in order, ahead of `base`, joined with a blank line;
    /// a provider returning `None` contributes nothing (but does not abort
    /// the fold), and an all-whitespace contribution is trimmed to nothing.
    #[tokio::test]
    async fn providers_fold_in_order_ahead_of_base() {
        let keys = nostr::Keys::generate();
        let c = ctx(&keys);
        let providers: Vec<Box<dyn TeamContextProvider>> = vec![
            Box::new(StubProvider("first", Some("First section."))),
            Box::new(StubProvider("silent", None)),
            Box::new(StubProvider("blank", Some("   "))),
            Box::new(StubProvider("second", Some("Second section."))),
        ];

        let out = build_team_instructions(&providers, &c, Some("Base.")).await;
        assert_eq!(
            out,
            Some("First section.\n\nSecond section.\n\nBase.".to_string())
        );
    }

    /// All providers silent and no base yields `None`, not an empty string.
    #[tokio::test]
    async fn all_empty_yields_none() {
        let keys = nostr::Keys::generate();
        let c = ctx(&keys);
        let providers: Vec<Box<dyn TeamContextProvider>> =
            vec![Box::new(StubProvider("silent", None))];

        assert_eq!(build_team_instructions(&providers, &c, None).await, None);
    }
}
