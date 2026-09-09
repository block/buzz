use super::*;
use std::cell::RefCell;
use std::collections::BTreeSet;

pub(super) struct History<'a> {
    pub records: &'a BTreeMap<String, Record>,
    invalid: &'a BTreeMap<String, Fault>,
    pub trust: &'a Trust,
    pub evidence: &'a Evidence,
    pub now: u64,
    cache: RefCell<BTreeMap<String, Check>>,
    visiting: RefCell<BTreeSet<String>>,
}
impl<'a> History<'a> {
    pub fn new(
        records: &'a BTreeMap<String, Record>,
        invalid: &'a BTreeMap<String, Fault>,
        trust: &'a Trust,
        evidence: &'a Evidence,
        now: u64,
    ) -> Self {
        Self {
            records,
            invalid,
            trust,
            evidence,
            now,
            cache: RefCell::new(BTreeMap::new()),
            visiting: RefCell::new(BTreeSet::new()),
        }
    }
    pub fn get(&self, id: &str) -> Check<&'a Record> {
        self.records.get(id).ok_or_else(|| {
            if self.invalid.contains_key(id) {
                fail("references", "invalid-reference")
            } else {
                pending("references", "missing-reference")
            }
        })
    }
    pub fn reference(
        &self,
        e: &Record,
        id: &str,
        typ: &str,
        code: &'static str,
    ) -> Check<&'a Record> {
        let r = self.get(id)?;
        if !typ.split('|').any(|t| t == r.typ()) {
            return Err(fail("references", code));
        }
        if r.typ() != "repository" && r.tag("a") != e.tag("a") {
            return Err(fail("references", "repository"));
        }
        if r.time() > e.time() {
            return Err(fail("references", "reference-time"));
        }
        match self.historical(id) {
            Ok(()) => Ok(r),
            Err(f) if f.outcome == "pending" => Err(f),
            Err(_) => Err(fail("references", "invalid-reference")),
        }
    }
    pub fn historical(&self, id: &str) -> Check {
        if let Some(v) = self.cache.borrow().get(id) {
            return v.clone();
        }
        if !self.visiting.borrow_mut().insert(id.to_owned()) {
            return Err(fail("references", "cycle"));
        }
        let result = self.get(id).and_then(|e| self.check(e));
        self.visiting.borrow_mut().remove(id);
        self.cache
            .borrow_mut()
            .insert(id.to_owned(), result.clone());
        result
    }
    pub fn valid(&self) -> Vec<&'a Record> {
        self.records
            .values()
            .filter(|r| self.historical(r.id()).is_ok())
            .collect()
    }
    pub fn before(&self, e: &Record, typ: &str) -> Vec<&'a Record> {
        self.records
            .values()
            .filter(|r| {
                r.id() != e.id()
                    && r.time() <= e.time()
                    && r.typ() == typ
                    && !self.depends_on(r, e.id())
                    && !self.visiting.borrow().contains(r.id())
                    && self.historical(r.id()).is_ok()
            })
            .collect()
    }
    pub fn selected(&self, e: &Record, typ: &str, issue: &str) -> Check<Option<&'a Record>> {
        let candidates: Vec<_> = self
            .before(e, typ)
            .into_iter()
            .filter(|r| r.issue() == issue)
            .collect();
        let parents: BTreeSet<_> = candidates.iter().map(|r| r.previous()).collect();
        let heads: Vec<_> = candidates
            .iter()
            .filter(|r| !parents.contains(r.id()))
            .copied()
            .collect();
        if heads.len() > 1 {
            return Err(Fault {
                outcome: "conflict",
                stage: "causality",
                code: "head-conflict",
            });
        }
        Ok(heads.first().copied())
    }
    // Transitive explicit-reference reachability, independent of delivery order.
    pub fn depends_on(&self, e: &Record, id: &str) -> bool {
        let mut todo = self.dependencies(e);
        let mut seen = BTreeSet::new();
        while let Some(dep) = todo.pop() {
            if dep == id {
                return true;
            }
            if seen.insert(dep.clone()) {
                if let Some(r) = self.records.get(&dep) {
                    todo.extend(self.dependencies(r));
                }
            }
        }
        false
    }
    pub fn descendant(&self, child: &Record, ancestor: &str) -> bool {
        let mut cur = child.previous();
        let mut seen = BTreeSet::new();
        while !cur.is_empty() && seen.insert(cur) {
            if cur == ancestor {
                return true;
            }
            cur = self.records.get(cur).map(|r| r.previous()).unwrap_or("");
        }
        false
    }
    // A competing branch is operationally disputed, but cannot invalidate the
    // exact historical branch. A linear supersession at authored time still can.
    pub fn current_binding(&self, e: &Record, bound: &Record, code: &'static str) -> Check {
        if self
            .before(e, bound.typ())
            .iter()
            .any(|r| self.chain_key(r) == self.chain_key(bound) && self.descendant(r, bound.id()))
        {
            return Err(fail("causality", code));
        }
        Ok(())
    }
    pub fn chain_key(&self, e: &Record) -> String {
        let scope = match e.typ() {
            "role-policy" => e.tag("a").to_owned(),
            "issue-update" | "triage-action" | "issue-state" | "assignment" => e.issue().to_owned(),
            "issue-relation" => {
                let mut ends = [e.issue(), e.field("target")];
                if e.field("relation") == "related" {
                    ends.sort();
                }
                format!("{}:{}:{}", ends[0], e.field("relation"), ends[1])
            }
            "release-set" if e.field("action") == "close" => e.field("set").to_owned(),
            "test-ready" => e.field("set").to_owned(),
            "build-run" => e.field("request").to_owned(),
            "build-request" => self
                .records
                .get(e.field("set"))
                .map(|s| s.field("pipeline"))
                .unwrap_or("")
                .to_owned(),
            "member-verdict" => {
                format!("{}:{}:{}", e.field("set"), e.field("test_ready"), e.issue())
            }
            _ => e.id().to_owned(),
        };
        format!("{}:{scope}", e.typ())
    }
    pub fn conflicts(&self) -> BTreeMap<String, &'static str> {
        let valid = self.valid();
        let mut groups: BTreeMap<(String, String), Vec<&Record>> = BTreeMap::new();
        for e in &valid {
            groups
                .entry((self.chain_key(e), e.previous().into()))
                .or_default()
                .push(e);
        }
        let mut result = BTreeMap::new();
        for g in groups.values().filter(|g| g.len() > 1) {
            let code = match g[0].typ() {
                "member-verdict" => "verdict-conflict",
                "build-request" => "build-lock",
                "release-set" => "head-conflict",
                _ => "fork",
            };
            for r in g {
                result.insert(r.id().into(), code);
            }
        }
        let sets: Vec<_> = valid
            .iter()
            .filter(|e| e.typ() == "release-set" && e.field("action") == "freeze")
            .copied()
            .collect();
        for (i, l) in sets.iter().enumerate() {
            for r in &sets[i + 1..] {
                let at = l.time().max(r.time());
                if !self.terminal_at(l.id(), at, &valid)
                    && !self.terminal_at(r.id(), at, &valid)
                    && a(&l.body["members"]).iter().any(|m| {
                        a(&r.body["members"])
                            .iter()
                            .any(|n| m["issue"] == n["issue"])
                    })
                {
                    result.insert(l.id().into(), "active-membership");
                    result.insert(r.id().into(), "active-membership");
                }
            }
        }
        result
    }
    pub fn terminal_at(&self, set: &str, at: u64, valid: &[&Record]) -> bool {
        self.set_status(set, at, valid) != "active"
    }
    pub fn set_status(&self, set: &str, at: u64, valid: &[&Record]) -> &'static str {
        let Some(freeze) = self.records.get(set) else {
            return "active";
        };
        let rows: Vec<_> = valid
            .iter()
            .filter(|r| r.time() <= at && r.field("set") == set)
            .collect();
        if !a(&freeze.body["members"]).is_empty()
            && a(&freeze.body["members"]).iter().all(|m| {
                rows.iter()
                    .any(|r| r.typ() == "member-verdict" && r.issue() == s(&m["issue"]))
            })
        {
            return "completed";
        }
        if rows
            .iter()
            .any(|r| r.typ() == "release-set" && r.field("outcome") == "failed")
        {
            return "failed";
        }
        if rows
            .iter()
            .any(|r| r.typ() == "release-set" && r.field("outcome") == "aborted")
        {
            return "aborted";
        }
        "active"
    }
    pub fn operationally_blocked(
        &self,
        e: &Record,
        conflicts: &BTreeMap<String, &'static str>,
    ) -> bool {
        if e.typ() == "assignment"
            && conflicts.keys().any(|id| {
                self.records.get(id).is_some_and(|p| {
                    p.typ() == "role-policy" && n(&p.body["effective_at"]) <= e.time()
                })
            })
        {
            return true;
        }
        let mut todo = vec![e.id().to_owned()];
        let mut seen = BTreeSet::new();
        while let Some(id) = todo.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if conflicts.contains_key(&id) {
                return true;
            }
            if let Some(r) = self.records.get(&id) {
                todo.extend(self.dependencies(r));
            }
        }
        false
    }
    pub fn validate(&self, id: &str) -> Check {
        let e = self.get(id)?;
        self.historical(id)?;
        if e.legacy()
            || !matches!(
                e.typ(),
                "repository"
                    | "root"
                    | "assignment"
                    | "artifact"
                    | "role-policy"
                    | "issue-update"
                    | "triage-action"
                    | "issue-state"
                    | "issue-relation"
                    | "release-pipeline"
                    | "release-set"
                    | "build-request"
                    | "build-run"
                    | "test-ready"
                    | "member-verdict"
            )
        {
            return Err(Fault {
                outcome: "ignore",
                stage: "projection",
                code: "legacy",
            });
        }
        let conflicts = self.conflicts();
        if let Some(code) = conflicts.get(id) {
            return Err(Fault {
                outcome: "conflict",
                stage: "causality",
                code,
            });
        }
        // Historical review and close folds survive operational conflicts. All other
        // consumers must block if an explicit dependency belongs to a disputed chain.
        if !(matches!(e.typ(), "member-verdict" | "test-ready")
            || e.typ() == "release-set" && e.field("action") == "close")
            && self.operationally_blocked(e, &conflicts)
        {
            return Err(Fault {
                outcome: "conflict",
                stage: "causality",
                code: "head-conflict",
            });
        }
        Ok(())
    }
    pub fn dependencies(&self, e: &Record) -> Vec<String> {
        let mut ids: Vec<_> = [
            e.tag("policy"),
            e.previous(),
            e.issue(),
            e.tag("set"),
            e.tag("run"),
        ]
        .into_iter()
        .filter(|x| !x.is_empty())
        .map(str::to_owned)
        .collect();
        for k in [
            "triage",
            "assignment",
            "update",
            "rework",
            "terminal_set",
            "target",
            "pipeline",
            "set",
            "request",
            "retry_of",
            "run",
            "test_ready",
            "artifact",
        ] {
            if let Some(s) = e.body[k].as_str() {
                ids.push(s.into());
            }
        }
        ids.extend(a(&e.body["artifacts"]).iter().map(|x| s(x).to_owned()));
        for m in a(&e.body["members"]) {
            ids.push(s(&m["issue"]).into());
            ids.push(s(&m["implemented"]).into());
        }
        ids
    }
}
