use super::history::History;
use super::*;
use sha2::{Digest, Sha256};

impl History<'_> {
    pub(super) fn external(&self, e: &Record) -> Check {
        let err = |code| fail("external", code);
        if e.typ() == "issue-state" && e.field("state") == "implemented"
            || e.typ() == "release-set" && e.field("action") == "freeze"
        {
            let rb = &e.body[if e.typ() == "issue-state" {
                "remote_readback"
            } else {
                "readback"
            }];
            let mut errors = Vec::new();
            if n(&rb["observed_at"]) > e.time()
                || e.time().saturating_sub(n(&rb["observed_at"])) > 300
            {
                errors.push(err("relay-head"));
            }
            if self.evidence.git_readbacks.is_empty() {
                errors.push(pending("external", "relay-head"));
            } else if !self.evidence.git_readbacks.contains(rb)
                || self.evidence.git_readbacks.iter().any(|r| {
                    r["repo"] == rb["repo"]
                        && r["stream"] == rb["stream"]
                        && r["observed_at"] == rb["observed_at"]
                        && r["head"] != rb["head"]
                })
            {
                errors.push(err("relay-head"));
            }
            if e.typ() == "release-set" {
                if rb["repo"] != e.tag("a")
                    || rb["stream"] != e.body["stream"]
                    || rb["head"] != e.body["relay_sha"]
                {
                    errors.push(err("relay-head"));
                }
                for m in a(&e.body["members"]) {
                    let commit = self.get(s(&m["implemented"]))?.field("commit");
                    let facts: Vec<_> = self
                        .evidence
                        .git_ancestry
                        .iter()
                        .filter(|a| {
                            s(&a["ancestor"]) == commit && a["descendant"] == e.body["relay_sha"]
                        })
                        .collect();
                    if facts.is_empty() {
                        errors.push(pending("external", "git-ancestry"));
                    } else if facts.iter().any(|f| f["is_ancestor"] != true) {
                        errors.push(err("git-ancestry"));
                    }
                }
            }
            first_failure(errors)?;
        }
        if e.typ() == "build-run" {
            let pipe = self.pipeline(e)?.ok_or_else(|| err("provider-readback"))?;
            if e.field("provider") != pipe.field("provider")
                || n(&e.body["observed_at"]) > e.time()
                || e.time() - n(&e.body["observed_at"]) > 300
            {
                return Err(err("provider-readback"));
            }
            if self.evidence.provider_readbacks.is_empty() {
                return Err(pending("external", "provider-readback"));
            }
            if !self.evidence.provider_readbacks.iter().any(|r| {
                r["idempotency_key"] == e.body["request"]
                    && e.body
                        .as_object()
                        .is_some_and(|b| b.iter().all(|(k, v)| r[k] == *v))
            }) {
                return Err(err("provider-readback"));
            }
        }
        if e.typ() == "artifact" {
            let row = self
                .evidence
                .downloads
                .iter()
                .find(|d| s(&d["url"]) == e.tag("url"))
                .ok_or_else(|| pending("external", "artifact-download"))?;
            if self
                .evidence
                .downloads
                .iter()
                .any(|d| d["url"] == row["url"] && d["bytes_hex"] != row["bytes_hex"])
            {
                return Err(err("artifact-digest"));
            }
            let bytes = hex::decode(s(&row["bytes_hex"])).map_err(|_| err("artifact-digest"))?;
            if hex::encode(Sha256::digest(&bytes)) != e.tag("x")
                || bytes.len().to_string() != e.tag("size")
            {
                return Err(err("artifact-digest"));
            }
        }
        if e.typ() == "test-ready" {
            let installation = &e.body["installation"];
            let set = self.get(e.field("set"))?;
            let windows = set.field("platform") == "windows";
            if installation["immutable"] != true
                || installation["method"]
                    != if windows {
                        "tailnet-download"
                    } else {
                        "download"
                    }
                || (windows
                    && (installation["unsigned"] != true
                        || !e.field("limitations").to_lowercase().contains("unsigned")))
            {
                return Err(err("durability"));
            }
            if !a(&e.body["artifacts"]).iter().any(|id| {
                self.records
                    .get(s(id))
                    .is_some_and(|r| r.tag("url") == s(&installation["url"]))
            }) {
                return Err(err("durability"));
            }
            let row = self
                .evidence
                .downloads
                .iter()
                .find(|d| d["url"] == installation["url"])
                .ok_or_else(|| pending("external", "durability"))?;
            if self
                .evidence
                .downloads
                .iter()
                .filter(|d| d["url"] == row["url"])
                .any(|d| {
                    d["immutable"] != true
                        || d["ephemeral"] != false
                        || windows && d["tailnet"] != true
                })
            {
                return Err(err("durability"));
            }
        }
        Ok(())
    }
}
