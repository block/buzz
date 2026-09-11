use super::history::History;
use super::*;
impl<'a> History<'a> {
    pub fn fields_at(&self, issue: &str, at: u64) -> Value {
        let updates: Vec<_> = self
            .records
            .values()
            .filter(|r| {
                r.typ() == "issue-update"
                    && r.issue() == issue
                    && r.time() <= at
                    && self.historical(r.id()).is_ok()
            })
            .collect();
        // Follow explicit ancestry, never a timestamp winner. Multiple heads expose
        // a conflict elsewhere and withhold the disputed fields here.
        let heads: Vec<_> = updates
            .iter()
            .filter(|r| !updates.iter().any(|s| s.previous() == r.id()))
            .collect();
        self.fields_from(
            issue,
            if heads.len() == 1 {
                Some(*heads[0])
            } else {
                None
            },
        )
    }
    pub fn fields_from(&self, issue: &str, head: Option<&Record>) -> Value {
        let root = self.records.get(issue);
        let mut fields = json!({"title":root.map(|r|r.tag("subject")).unwrap_or(""),"description":root.map(|r|s(&r.wire["content"])).unwrap_or(""),"acceptance_criteria":[],"non_goals":[],"type":"task","priority":"P2","platform":null});
        let mut chain = Vec::new();
        let mut cur = head;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(r) = cur {
            if !seen.insert(r.id()) {
                break;
            }
            chain.push(r);
            cur = self.records.get(r.previous());
        }
        for r in chain.into_iter().rev() {
            if let Some(patch) = r.body["patch"].as_object() {
                for (k, v) in patch {
                    fields[k] = v.clone();
                }
            }
        }
        fields
    }
    pub fn project(&self, _target: Option<&str>) -> Value {
        let valid = self.valid();
        let conflicts = self.conflicts();
        let operational: Vec<_> = valid
            .iter()
            .filter(|r| !self.operationally_blocked(r, &conflicts))
            .copied()
            .collect();
        let mut p = json!({"issues":{},"issue_fields":{},"issue_state":{},"issue_state_id":{},"leaf":{},"artifact_verdicts":{},"conflicts":conflicts.keys().collect::<Vec<_>>(),"children":{},"sets":{},"handoffs":{},"active_members":[],"dispatch_count":0,"building":false});
        for root in valid.iter().filter(|e| e.typ() == "root") {
            let states: Vec<_> = operational
                .iter()
                .filter(|r| r.typ() == "issue-state" && r.issue() == root.id())
                .collect();
            let heads: Vec<_> = states
                .iter()
                .filter(|r| !states.iter().any(|s| s.previous() == r.id()))
                .collect();
            let head = heads.first().filter(|_| heads.len() == 1);
            let mut state = head.map(|h| h.field("state")).unwrap_or("");
            let verdicts: Vec<_> = valid
                .iter()
                .filter(|r| r.typ() == "member-verdict" && r.issue() == root.id())
                .collect();
            if verdicts.iter().any(|v| v.field("verdict") == "accepted") {
                state = "resolved"
            } else if verdicts.iter().any(|v| {
                v.field("verdict") == "rejected" && head.is_none_or(|h| v.time() >= h.time())
            }) {
                state = "rework"
            } else if state == "triage" {
                let actions: Vec<_> = operational
                    .iter()
                    .filter(|r| r.typ() == "triage-action" && r.issue() == root.id())
                    .collect();
                let actions: Vec<_> = actions
                    .iter()
                    .filter(|r| !actions.iter().any(|s| s.previous() == r.id()))
                    .collect();
                if actions.len() == 1 {
                    state = match actions[0].field("action") {
                        "accept" => "backlog",
                        "duplicate" | "decline" => "closed",
                        _ => state,
                    };
                }
            }
            if state.is_empty() {
                continue;
            }
            p["issues"][root.id()] = json!(state);
            p["issue_fields"][root.id()] = self.fields_at(root.id(), self.now);
            // Literal passthrough of the current, unambiguous issue-state head's
            // already-accepted body (stream/assignment/commit/tests/remote_readback
            // as applicable to that state) and its own event ID — no new
            // authority decision, only display of one Core already made when
            // it accepted that event. The ID lets a producer chain the next
            // transition's `previous` tag without re-deriving this head itself.
            if let Some(h) = head {
                p["issue_state"][root.id()] = h.body.clone();
                p["issue_state_id"][root.id()] = json!(h.id());
            }
            p["leaf"][root.id()] = json!(self.is_executable_leaf(root.id(), &valid));
        }
        for e in valid.iter().filter(|r| r.typ() == "issue-relation") {
            if e.field("relation") == "parent-of" {
                p["children"][e.issue()] = json!([]);
            }
        }
        for r in self.relations(&operational) {
            if r.field("relation") == "parent-of" {
                if let Some(v) = p["children"][r.issue()].as_array_mut() {
                    v.push(json!(r.field("target")));
                }
            }
        }
        let mut relations = std::collections::BTreeSet::new();
        for r in self.relations(&operational) {
            let inverse = match r.field("relation") {
                "parent-of" => "child-of",
                "blocks" => "blocked-by",
                "duplicate-of" => "duplicates",
                _ => "related",
            };
            relations.insert((r.issue(), r.field("relation"), r.field("target")));
            relations.insert((r.field("target"), inverse, r.issue()));
        }
        for r in valid
            .iter()
            .filter(|r| r.typ() == "triage-action" && r.field("action") == "duplicate")
        {
            relations.insert((r.issue(), "duplicate-of", r.field("target")));
            relations.insert((r.field("target"), "duplicates", r.issue()));
        }
        p["relations"]=json!(relations.into_iter().map(|(issue,relation,target)|json!({"issue":issue,"relation":relation,"target":target})).collect::<Vec<_>>());
        let mut sets: Vec<_> = valid
            .iter()
            .filter(|r| r.typ() == "release-set" && r.field("action") == "freeze")
            .copied()
            .collect();
        sets.sort_by_key(|r| (r.time(), r.id()));
        let mut members = std::collections::BTreeSet::new();
        for set in &sets {
            let status = self.set_status(set.id(), self.now, &valid);
            p["sets"][set.id()] = json!(status);
            if status == "active" {
                for m in a(&set.body["members"]) {
                    members.insert(s(&m["issue"]));
                }
            }
            let handoffs: Vec<_> = operational
                .iter()
                .filter(|r| r.typ() == "test-ready" && r.field("set") == set.id())
                .collect();
            let heads: Vec<_> = handoffs
                .iter()
                .filter(|r| !handoffs.iter().any(|s| s.previous() == r.id()))
                .collect();
            if heads.len() == 1 {
                p["handoffs"][set.id()] = json!(heads[0].id());
            }
            for artifact in valid
                .iter()
                .filter(|r| r.typ() == "artifact" && r.tag("set") == set.id())
            {
                for m in a(&set.body["members"]) {
                    let issue = s(&m["issue"]);
                    let votes: std::collections::BTreeSet<_> = valid
                        .iter()
                        .filter(|r| {
                            r.typ() == "member-verdict"
                                && r.issue() == issue
                                && r.field("artifact") == artifact.id()
                        })
                        .map(|r| r.field("verdict"))
                        .collect();
                    p["artifact_verdicts"][artifact.id()][issue] = json!(if votes.len() > 1 {
                        "conflict"
                    } else {
                        votes.first().copied().unwrap_or("unreviewed")
                    });
                }
            }
        }
        p["active_members"] = json!(members);
        if let Some(set) = sets.last() {
            p["set"] = p["sets"][set.id()].clone();
            if !p["handoffs"][set.id()].is_null() {
                p["handoff"] = p["handoffs"][set.id()].clone();
            }
        }
        let requests: Vec<_> = operational
            .iter()
            .filter(|r| r.typ() == "build-request")
            .collect();
        p["host_authorized"] = json!(self.evidence.host_authorization["allowed"] == true);
        p["dispatch_count"] = json!(if self.evidence.host_authorization["allowed"] == true {
            requests.len()
        } else {
            0
        });
        p["building"] = json!(requests.iter().any(|req| !valid
            .iter()
            .any(|r| r.typ() == "build-run"
                && r.field("request") == req.id()
                && matches!(r.field("result"), "success" | "failure" | "cancelled"))));
        if let Some(req) = requests.iter().max_by_key(|r| n(&r.body["attempt"])) {
            p["attempt"] = req.body["attempt"].clone();
        }
        let roots: Vec<_> = valid.iter().filter(|e| e.typ() == "root").collect();
        let selected = if roots.len() == 1 {
            Some(roots[0].id())
        } else {
            None
        };
        if let Some(issue) = selected {
            let activity = valid
                .iter()
                .filter(|r| {
                    r.issue() == issue
                        && matches!(
                            r.typ(),
                            "issue-state"
                                | "issue-update"
                                | "triage-action"
                                | "assignment"
                                | "member-verdict"
                        )
                })
                .map(|r| r.time())
                .max();
            p["stale"] = json!(activity.is_some_and(|t| self.now >= t + 604800));
            p["snoozed"] = json!(valid.iter().any(|r| r.issue() == issue
                && r.typ() == "triage-action"
                && r.field("action") == "snooze"
                && self.now < n(&r.body["until"])
                && !valid.iter().any(|s| s.previous() == r.id())));
        }
        p
    }
}
