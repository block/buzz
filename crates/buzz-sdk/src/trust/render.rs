//! A readable view of a [`WorkTrail`]: request → contribution(s) →
//! outcome(s), with exact event ids, qualifiers, gaps, and coverage.
//!
//! The text adds nothing the trail does not hold. It names every source by
//! full event id and key, prints every outcome beside the exact contribution
//! it judges, and ends with what could not be seen.

use std::fmt::Write;

use super::{
    AuthorityStatus, Contribution, ContributionRole, Independence, Outcome, OutcomeScope,
    OwnerLink, RefStatus, Task, WorkTrail,
};

impl WorkTrail {
    /// Render the trail for a reader. Plain text, one fact per line.
    pub fn readable(&self) -> String {
        let mut out = String::new();
        let s = &self.scope;
        let list = |v: &[String]| {
            if v.is_empty() {
                "none".to_owned()
            } else {
                v.join(", ")
            }
        };
        let _ = writeln!(
            out,
            "work trail · policy {} (version {}) · from {}",
            self.policy.name, self.policy.version, self.entry
        );
        let _ = writeln!(
            out,
            "scope: origin {} · channels {} · repos {} · since {} · until {}",
            s.origin,
            list(&s.channels),
            list(&s.repos),
            s.since.map_or("unbounded".to_owned(), |t| t.to_string()),
            s.cutoff
        );
        if self.tasks.is_empty() {
            let _ = writeln!(out, "\nno work found in this queried scope");
        }
        for task in &self.tasks {
            task_text(&mut out, task);
        }
        let _ = writeln!(out, "\nwhat this view cannot show:");
        let _ = writeln!(
            out,
            "  - signatures bind bytes to keys; they do not show quality, distinct people, or true time"
        );
        let _ = writeln!(
            out,
            "  - artifact links are shown as signed; their content is not fetched or compared"
        );
        for gap in &self.gaps {
            match &gap.id {
                Some(id) => {
                    let _ = writeln!(out, "  - {id}: {}", gap.reason);
                }
                None => {
                    let _ = writeln!(out, "  - {}", gap.reason);
                }
            }
        }
        let _ = writeln!(out, "\ncoverage:");
        for line in &self.coverage {
            let _ = writeln!(out, "  - {line}");
        }
        let invalid = self.evidence.events.iter().filter(|e| !e.valid).count();
        let _ = writeln!(
            out,
            "\nevidence: {} signed events kept as served in the JSON output ({invalid} fail verification and support nothing)",
            self.evidence.events.len()
        );
        let _ = writeln!(
            out,
            "  `trust recheck` authenticates those events offline, not this view, its labels, provenance or coverage"
        );
        out
    }
}

fn task_text(out: &mut String, task: &Task) {
    let r = &task.request;
    let title = r.title.as_deref().unwrap_or("(no typed request)");
    let _ = write!(out, "\ntask: {title}");
    if let Some(kind) = &r.task_type {
        let _ = write!(out, " ({kind})");
    }
    let _ = writeln!(out);
    match (&r.id, &r.requester) {
        (Some(id), Some(by)) => {
            let at = r.created_at.map_or(String::new(), |t| format!(" at {t}"));
            let _ = writeln!(out, "  request {id} by {by}{at} · {}", r.status);
        }
        (Some(id), None) => {
            let _ = writeln!(out, "  request {id} · {}", r.status);
        }
        _ => {
            let _ = writeln!(out, "  request: {}", r.status);
        }
    }
    for c in &r.acceptance_criteria {
        let _ = writeln!(out, "    criterion: {c}");
    }
    let _ = writeln!(out, "  contributions:");
    if task.contributions.is_empty() && r.kind == Some(1621) {
        let _ = writeln!(
            out,
            "    none linked: no convention links pull requests to issues yet"
        );
    } else if task.contributions.is_empty() {
        let _ = writeln!(out, "    none observed in this scope");
    }
    for c in &task.contributions {
        contribution_text(out, c);
    }
    let (request, unbound): (Vec<&Outcome>, Vec<&Outcome>) = task
        .unbound
        .iter()
        .partition(|o| o.scope == OutcomeScope::Request);
    for (title, list) in [
        ("request statuses (judge no contribution):", request),
        ("outcomes that name no exact revision:", unbound),
    ] {
        if !list.is_empty() {
            let _ = writeln!(out, "  {title}");
            for o in list {
                outcome_text(out, o, "    ");
            }
        }
    }
    if !task.conversation.is_empty() {
        let _ = writeln!(out, "  conversation (not work, not acceptance):");
        for c in &task.conversation {
            let _ = writeln!(
                out,
                "    {} by {} at {} · {}",
                c.id, c.signer, c.created_at, c.note
            );
        }
    }
}

fn contribution_text(out: &mut String, c: &Contribution) {
    let role = match c.role {
        ContributionRole::Commitment => "commitment (claims responsibility, not completion)",
        ContributionRole::Update => "update (the contributor's statement)",
        ContributionRole::Submission => "submission",
        ContributionRole::Proposal => "pull request, revision one",
        ContributionRole::ProposalUpdate => "pull request update",
        ContributionRole::Patch => "patch",
    };
    let _ = writeln!(out, "    {role} {}", c.id);
    let owner = match &c.owner_link {
        OwnerLink::Verified(owner) => {
            format!(" · owner link to {owner} (credit stays with the signer)")
        }
        OwnerLink::Invalid => " · owner link invalid".to_owned(),
        OwnerLink::None => String::new(),
    };
    let _ = writeln!(
        out,
        "      signed by {} at {}{owner}",
        c.signer, c.created_at
    );
    if let Some(summary) = &c.summary {
        let _ = writeln!(out, "      says: {summary}");
    }
    if let Some(state) = &c.state {
        let _ = writeln!(out, "      state: {state}");
    }
    if let Some(artifact) = &c.artifact {
        let _ = writeln!(out, "      artifact: {artifact}");
    }
    if let Some(revision) = &c.revision {
        let _ = writeln!(out, "      revision: {revision}");
    }
    if let Some(previous) = &c.previous {
        let _ = writeln!(out, "      follows: {previous}");
    }
    if let Some(why) = &c.unresolved {
        let _ = writeln!(out, "      unresolved: {why}");
    }
    if c.outcomes.is_empty() {
        let _ = writeln!(out, "      outcome: none observed in this scope");
    }
    for o in &c.outcomes {
        outcome_text(out, o, "      ");
    }
}

fn outcome_text(out: &mut String, o: &Outcome, pad: &str) {
    let authority = match o.authority.status {
        AuthorityStatus::Authorized => "authorized",
        AuthorityStatus::Delegated => "delegated",
        AuthorityStatus::SelfAsserted => "self-asserted",
        AuthorityStatus::Opinion => "opinion",
        AuthorityStatus::Unknown => "authority unknown",
    };
    let relationship = match o.relationship {
        Independence::SameSigner => "same signer as the work",
        Independence::SameOwner => "same owner as the work",
        Independence::Unknown => "relationship unknown",
    };
    let _ = writeln!(
        out,
        "{pad}outcome {} {} by {} at {}",
        o.claim, o.id, o.signer, o.created_at
    );
    let _ = writeln!(
        out,
        "{pad}  {authority} ({}) · {relationship}",
        o.authority.basis
    );
    if let Some(note) = &o.note {
        let _ = writeln!(out, "{pad}  {note}");
    }
    if let Some(r) = &o.ref_state {
        let status = match r.status {
            RefStatus::Corroborated => "merge commit corroborated by ref state",
            RefStatus::Unverified => "merge commit not corroborated",
        };
        let _ = writeln!(out, "{pad}  {status} ({}); not review", r.basis);
    }
    if let Some(reason) = &o.reason {
        let _ = writeln!(out, "{pad}  reason: {reason}");
    }
    for c in &o.criteria_checked {
        let _ = writeln!(out, "{pad}  says it checked: {c}");
    }
    for e in &o.evidence_refs {
        let _ = writeln!(out, "{pad}  cites (not fetched): {e}");
    }
}
