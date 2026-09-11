use super::history::History;
use super::*;
use std::collections::BTreeSet;

impl<'a> History<'a> {
    pub fn check(&self, e: &Record) -> Check {
        if e.legacy() || e.typ() == "unrelated" {
            return Ok(());
        }
        if e.time() > self.now {
            return Err(pending("references", "future"));
        }
        if !shape::repo(&self.trust.repo)
            || self.trust.repo.split(':').nth(1) != Some(self.trust.owner.as_str())
            || !shape::url(&self.trust.community)
            || self.now > u32::MAX as u64
        {
            return Err(fail("references", "trust"));
        }
        if e.typ() == "repository" {
            if e.signer() != self.trust.owner
                || format!("30617:{}:{}", e.signer(), e.tag("d")) != self.trust.repo
            {
                return Err(fail("references", "repository"));
            }
            return Ok(());
        }
        if e.tag("a") != self.trust.repo {
            return Err(fail("references", "repository"));
        }
        if !self
            .records
            .values()
            .any(|r| r.typ() == "repository" && self.historical(r.id()).is_ok())
        {
            return Err(pending("references", "missing-reference"));
        }
        if e.typ() == "root" {
            return Ok(());
        }
        let mut errors = Vec::new();
        if !e.issue().is_empty() {
            if let Err(f) = self.reference(e, e.issue(), "root", "issue-root") {
                errors.push(f);
            }
        }
        let policy = self.policy(e);
        if let Err(f) = &policy {
            errors.push(f.clone());
        }
        if let Err(f) = self.bindings(e) {
            errors.push(f);
        }
        let pipeline = self.pipeline(e);
        if let Err(f) = &pipeline {
            errors.push(f.clone());
        }
        first_failure(errors)?;
        let p = policy?;
        // Policy checks precede role, causality and external facts.
        if e.typ() == "role-policy" {
            self.policy_rules(e)?;
        } else if let Some(p) = p {
            if e.typ() == "issue-state"
                && e.field("state") == "triage"
                && self.get(e.issue())?.time() < n(&p.body["cutover"])
            {
                return Err(fail("policy", "cutover"));
            }
            let pinned = pipeline?;
            if let Some(pipe) = pinned {
                if e.tag("policy") != pipe.tag("policy") && e.typ() != "artifact" {
                    return Err(fail("policy", "policy-binding"));
                }
            } else if e.typ() != "assignment"
                && (n(&p.body["effective_at"]) > e.time()
                    || self.before(e, "role-policy").iter().any(|q| {
                        n(&q.body["effective_at"]) <= e.time() && self.descendant(q, p.id())
                    }))
            {
                return Err(fail("policy", "policy-binding"));
            }
        }
        self.roles(e, p)?;
        self.causality(e, p)?;
        self.external(e)?;
        Ok(())
    }
    fn policy_heads(&self, e: &Record) -> Vec<&'a Record> {
        let policies: Vec<_> = self
            .before(e, "role-policy")
            .into_iter()
            .filter(|p| n(&p.body["effective_at"]) <= e.time())
            .collect();
        policies
            .iter()
            .filter(|p| !policies.iter().any(|q| self.descendant(q, p.id())))
            .copied()
            .collect()
    }
    fn assignment_authority(&self, assignment: &Record, policy: &Record) -> bool {
        let mut p = policy;
        let mut seen = BTreeSet::new();
        while n(&p.body["effective_at"]) > assignment.time() {
            if !seen.insert(p.id()) {
                return false;
            }
            let Some(parent) = self.records.get(p.previous()) else {
                return false;
            };
            p = parent;
        }
        assignment.signer() == self.trust.owner
            || a(&p.body["coordinators"])
                .iter()
                .any(|k| s(k) == assignment.signer())
    }
    fn policy(&self, e: &Record) -> Check<Option<&'a Record>> {
        if e.typ() == "role-policy" && e.previous().is_empty() {
            return Ok(None);
        }
        if e.typ() == "assignment" {
            if self.policy_heads(e).is_empty() {
                return Err(pending("policy", "missing-policy"));
            }
            return Ok(None);
        }
        if e.typ() == "artifact" {
            let set = self.reference(e, e.tag("set"), "release-set", "set-reference")?;
            return self
                .reference(e, set.tag("policy"), "role-policy", "policy-reference")
                .map(Some);
        }
        self.reference(e, e.tag("policy"), "role-policy", "policy-reference")
            .map(Some)
    }
    fn policy_rules(&self, e: &Record) -> Check {
        if n(&e.body["effective_at"]) < e.time() {
            return Err(fail("policy", "effective-time"));
        }
        if e.previous().is_empty() {
            if n(&e.body["version"]) != 1 || n(&e.body["cutover"]) < n(&e.body["effective_at"]) {
                return Err(fail("policy", "policy-version"));
            }
        } else {
            let p = self.reference(e, e.previous(), "role-policy", "wrong-previous")?;
            if e.tag("policy") != p.id()
                || n(&e.body["version"]) != n(&p.body["version"]) + 1
                || e.body["cutover"] != p.body["cutover"]
                || n(&e.body["effective_at"]) <= n(&p.body["effective_at"])
            {
                return Err(fail("policy", "policy-version"));
            }
        }
        let mut delegations = BTreeSet::new();
        for d in a(&e.body["triage_delegations"]) {
            if !delegations.insert((s(&d["issue"]), s(&d["action"]), s(&d["delegate"]))) {
                return Err(fail("policy", "delegation"));
            }
        }
        Ok(())
    }
    pub fn pipeline(&self, e: &Record) -> Check<Option<&'a Record>> {
        let set = match e.typ() {
            "release-set" if e.field("action") == "freeze" => {
                return self
                    .reference(
                        e,
                        e.field("pipeline"),
                        "release-pipeline",
                        "pipeline-reference",
                    )
                    .map(Some)
            }
            "release-set" | "build-request" | "test-ready" | "member-verdict" => e.field("set"),
            "artifact" => e.tag("set"),
            "build-run" => self
                .reference(e, e.field("request"), "build-request", "request-reference")?
                .field("set"),
            _ => return Ok(None),
        };
        let set = self.reference(e, set, "release-set", "set-reference")?;
        if set.field("action") != "freeze" {
            return Err(fail("references", "set-reference"));
        }
        self.reference(
            e,
            set.field("pipeline"),
            "release-pipeline",
            "pipeline-reference",
        )
        .map(Some)
    }
    fn bindings(&self, e: &Record) -> Check {
        let mut errors = Vec::new();
        for id in self.dependencies(e) {
            if id == e.issue() || id == e.tag("policy") {
                continue;
            }
            if let Err(f)=self.reference(e,&id,"role-policy|issue-state|issue-update|triage-action|assignment|release-pipeline|release-set|build-request|build-run|test-ready|member-verdict|artifact|root|issue-relation","invalid-reference"){errors.push(f);}
        }
        for (field, typ, code) in [
            ("target", "root", "issue-root"),
            ("assignment", "assignment", "issue-reference"),
            ("update", "issue-update", "issue-reference"),
            ("triage", "triage-action", "issue-reference"),
            ("rework", "member-verdict", "issue-reference"),
            ("terminal_set", "release-set", "set-reference"),
            ("retry_of", "build-run", "run-reference"),
            ("test_ready", "test-ready", "artifact-reference"),
            ("artifact", "artifact", "artifact-reference"),
        ] {
            if let Some(id) = e.body[field].as_str() {
                if let Err(f) = self.reference(e, id, typ, code) {
                    errors.push(f);
                }
            }
        }
        let mut push = |ok: bool, code| {
            if !ok {
                errors.push(fail("references", code))
            }
        };
        if !e.previous().is_empty() {
            if let Ok(p) = self.get(e.previous()) {
                let valid = if e.typ() == "release-set" {
                    p.typ() == "release-set"
                        && (p.id() == e.field("set") || p.field("set") == e.field("set"))
                } else {
                    p.typ() == e.typ() && self.chain_key(p) == self.chain_key(e)
                };
                push(valid, "wrong-previous");
            }
        }
        match e.typ() {
            "issue-state" => {
                for (field, typ) in [
                    ("triage", "triage-action"),
                    ("update", "issue-update"),
                    ("assignment", "assignment"),
                    ("rework", "member-verdict"),
                ] {
                    if let Some(id) = e.body[field].as_str() {
                        if let Ok(r) = self.get(id) {
                            push(r.typ() == typ && r.issue() == e.issue(), "issue-reference");
                        }
                    }
                }
                if e.field("state") == "implemented" {
                    push(
                        e.body["remote_readback"]["repo"] == e.tag("a")
                            && e.body["remote_readback"]["head"] == e.body["commit"]
                            && e.body["remote_readback"]["stream"] == e.body["stream"],
                        "readback-reference",
                    );
                }
            }
            "release-set" if e.field("action") == "freeze" => {
                if let Ok(pipe) = self.get(e.field("pipeline")) {
                    push(pipe.typ() == "release-pipeline", "pipeline-reference");
                    push(pipe.body["stream"] == e.body["stream"], "stream");
                    push(pipe.body["platform"] == e.body["platform"], "platform");
                }
                for m in a(&e.body["members"]) {
                    if let Ok(imp) = self.get(s(&m["implemented"])) {
                        push(
                            imp.typ() == "issue-state"
                                && imp.field("state") == "implemented"
                                && imp.issue() == s(&m["issue"]),
                            "implemented",
                        );
                        push(imp.body["stream"] == e.body["stream"], "stream");
                        let mut cursor = Some(imp);
                        let mut seen = BTreeSet::new();
                        let mut update = None;
                        while let Some(state) = cursor {
                            if !seen.insert(state.id()) {
                                break;
                            }
                            if state.field("state") == "ready" {
                                update = self.records.get(state.field("update"));
                                break;
                            }
                            cursor = self.records.get(state.previous());
                        }
                        push(
                            update.is_some_and(|u| {
                                self.fields_from(imp.issue(), Some(u))["platform"]
                                    == e.body["platform"]
                            }),
                            "platform",
                        );
                    }
                }
            }
            "build-run" => {
                if let Ok(req) = self.get(e.field("request")) {
                    if let Ok(set) = self.get(req.field("set")) {
                        push(e.body["head"] == set.body["relay_sha"], "run-head");
                        push(e.body["attempt"] == req.body["attempt"], "run-attempt");
                    }
                }
            }
            "artifact" | "test-ready" => {
                let set_id = if e.typ() == "artifact" {
                    e.tag("set")
                } else {
                    e.field("set")
                };
                let run_id = if e.typ() == "artifact" {
                    e.tag("run")
                } else {
                    e.field("run")
                };
                if let Ok(run) = self.get(run_id) {
                    push(
                        run.typ() == "build-run" && run.field("result") == "success",
                        "run-reference",
                    );
                    if let Ok(req) = self.get(run.field("request")) {
                        push(req.field("set") == set_id, "run-reference");
                    }
                }
                if e.typ() == "artifact" {
                    if let Ok(set) = self.get(set_id) {
                        push(e.tag("release") == set.field("release"), "release");
                        push(e.tag("platform") == set.field("platform"), "platform");
                    }
                } else {
                    for id in a(&e.body["artifacts"]) {
                        if let Ok(art) = self.get(s(id)) {
                            push(
                                art.typ() == "artifact"
                                    && art.tag("set") == set_id
                                    && art.tag("run") == run_id,
                                "artifact-reference",
                            );
                        }
                    }
                }
            }
            "member-verdict" => {
                if let Ok(set) = self.get(e.field("set")) {
                    push(
                        a(&set.body["members"])
                            .iter()
                            .any(|m| s(&m["issue"]) == e.issue()),
                        "member",
                    );
                }
                if let Ok(h) = self.get(e.field("test_ready")) {
                    push(
                        h.typ() == "test-ready"
                            && h.field("set") == e.field("set")
                            && a(&h.body["artifacts"]).contains(&e.body["artifact"]),
                        "artifact-reference",
                    );
                }
                if let Ok(art) = self.get(e.field("artifact")) {
                    push(
                        art.typ() == "artifact" && art.tag("set") == e.field("set"),
                        "artifact-reference",
                    );
                }
            }
            _ => {}
        }
        errors.sort_by_key(|f| f.code);
        errors.into_iter().next().map_or(Ok(()), Err)
    }
    fn roles(&self, e: &Record, p: Option<&Record>) -> Check {
        let owner = e.signer() == self.trust.owner;
        let has =
            |role: &str| p.is_some_and(|p| a(&p.body[role]).iter().any(|v| s(v) == e.signer()));
        let coordinator = owner || has("coordinators");
        let authorized = match e.typ() {
            "role-policy" | "release-pipeline" => owner,
            "assignment" => self
                .policy_heads(e)
                .iter()
                .any(|p| self.assignment_authority(e, p)),
            "issue-relation" | "release-set" | "build-request" => coordinator,
            "issue-update" => {
                let reporter = self.get(e.issue())?.signer() == e.signer();
                if !(coordinator || has("operators") || reporter) {
                    false
                } else if ["type", "priority", "platform"]
                    .iter()
                    .any(|k| e.body["patch"].get(k).is_some())
                    && !(coordinator || has("operators"))
                {
                    return Err(fail("role", "field-permission"));
                } else {
                    true
                }
            }
            "triage-action" => {
                owner
                    || (has("operators") && matches!(e.field("action"), "need-info" | "snooze"))
                    || p.is_some_and(|p| {
                        e.tag("delegation") == p.id()
                            && a(&p.body["triage_delegations"]).iter().any(|d| {
                                s(&d["issue"]) == e.issue()
                                    && s(&d["action"]) == e.field("action")
                                    && s(&d["delegate"]) == e.signer()
                                    && e.time() < n(&d["expires_at"])
                            })
                    })
            }
            "issue-state" => match e.field("state") {
                "triage" => owner || self.get(e.issue())?.signer() == e.signer(),
                "backlog" | "ready" => coordinator,
                _ => self
                    .get(e.field("assignment"))
                    .is_ok_and(|r| r.tag("t") == "assignment" && r.tag("p") == e.signer()),
            },
            "build-run" => self.get(e.field("request"))?.field("worker") == e.signer(),
            "artifact" => self
                .pipeline(e)?
                .is_some_and(|p| p.field("publisher") == e.signer()),
            "test-ready" => {
                let run = self.get(e.field("run"))?;
                self.get(run.field("request"))?.field("worker") == e.signer()
            }
            "member-verdict" => {
                if !has("human_testers") {
                    false
                } else if self.get(e.field("test_ready"))?.field("tester") != e.signer() {
                    return Err(fail("role", "tester"));
                } else {
                    true
                }
            }
            _ => false,
        };
        if !authorized {
            return Err(fail("role", "unauthorized"));
        }
        if e.typ() == "release-pipeline"
            && !p.is_some_and(|p| {
                a(&p.body["workers"][e.field("platform")]).contains(&e.body["publisher"])
            })
        {
            return Err(fail("role", "publisher"));
        }
        if e.typ() == "build-request" {
            let pipe = self
                .pipeline(e)?
                .ok_or_else(|| fail("references", "pipeline-reference"))?;
            if !p.is_some_and(|p| {
                a(&p.body["workers"][pipe.field("platform")]).contains(&e.body["worker"])
            }) {
                return Err(fail("role", "worker"));
            }
        }
        if e.typ() == "test-ready"
            && !p.is_some_and(|p| a(&p.body["human_testers"]).contains(&e.body["tester"]))
        {
            return Err(fail("role", "tester"));
        }
        Ok(())
    }
    fn causality(&self, e: &Record, _p: Option<&Record>) -> Check {
        let err = |code| fail("causality", code);
        let previous = if e.previous().is_empty() {
            None
        } else {
            Some(self.get(e.previous())?)
        };
        match e.typ() {
            "issue-update" | "triage-action" | "assignment" | "issue-relation" => {
                let state = self
                    .selected(e, "issue-state", e.issue())?
                    .ok_or_else(|| err("not-enrolled"))?;
                if e.typ() == "issue-update" {
                    let class = ["type", "priority", "platform"]
                        .iter()
                        .any(|k| e.body["patch"].get(k).is_some());
                    if (class && !matches!(state.field("state"), "triage" | "backlog"))
                        || (!class
                            && matches!(state.field("state"), "in-development" | "implemented"))
                    {
                        return Err(err("update-state"));
                    }
                }
                if e.typ() == "triage-action" {
                    if state.field("state") != "triage" {
                        return Err(err("triage-state"));
                    }
                    if previous.is_some_and(|p| {
                        matches!(p.field("action"), "accept" | "decline" | "duplicate")
                    }) {
                        return Err(err("triage-terminal"));
                    }
                    if e.field("action") == "snooze"
                        && !(e.time() < n(&e.body["until"])
                            && n(&e.body["until"]) <= e.time() + 2592000)
                    {
                        return Err(err("snooze"));
                    }
                    if e.field("action") == "duplicate" && e.field("target") == e.issue() {
                        return Err(err("self-edge"));
                    }
                    if !e.tag("delegation").is_empty() && previous.is_some() {
                        return Err(err("delegation-head"));
                    }
                }
                if e.typ() == "assignment" {
                    let empty = previous.is_none_or(|p| p.tag("t") == "unassignment");
                    if e.tag("t") == "assignment" && !empty
                        || e.tag("t") == "unassignment"
                            && (empty || previous.is_none_or(|p| p.tag("p") != e.tag("p")))
                    {
                        return Err(err("assignment-operation"));
                    }
                }
                if e.typ() == "issue-relation" {
                    if e.field("target") == e.issue() {
                        return Err(err("self-edge"));
                    }
                    if e.field("relation") == "related" && e.issue() > e.field("target") {
                        return Err(err("relation-order"));
                    }
                    if self
                        .selected(e, "issue-state", e.field("target"))?
                        .is_none()
                    {
                        return Err(err("not-enrolled"));
                    }
                    if previous.is_none() && e.field("operation") != "add"
                        || previous.is_some_and(|p| p.field("operation") == e.field("operation"))
                    {
                        return Err(err("relation-operation"));
                    }
                    if e.field("operation") == "add" && e.field("relation") != "related" {
                        let valid = self.known_before(e);
                        if valid.iter().any(|r| {
                            r.typ() == "release-set"
                                && r.field("action") == "freeze"
                                && !self.terminal_at(r.id(), e.time(), &valid)
                                && a(&r.body["members"]).iter().any(|m| {
                                    s(&m["issue"]) == e.issue()
                                        || s(&m["issue"]) == e.field("target")
                                })
                        }) {
                            return Err(err("active-membership"));
                        }
                        if matches!(e.field("relation"), "parent-of" | "blocks") {
                            let edges = self.relations(&valid);
                            let mut todo = vec![e.field("target")];
                            let mut seen = BTreeSet::new();
                            while let Some(node) = todo.pop() {
                                if node == e.issue() {
                                    return Err(err("relation-cycle"));
                                }
                                if seen.insert(node) {
                                    for r in &edges {
                                        if r.issue() == node
                                            && r.field("relation") == e.field("relation")
                                        {
                                            todo.push(r.field("target"));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "issue-state" => {
                let state = e.field("state");
                if state == "triage" {
                    if previous.is_some() {
                        return Err(err("state-transition"));
                    }
                } else {
                    let p = previous.ok_or_else(|| err("wrong-previous"))?;
                    let allowed = match state {
                        "backlog" => {
                            p.field("state") == "triage"
                                && self.get(e.field("triage"))?.field("action") == "accept"
                        }
                        "ready" => matches!(
                            p.field("state"),
                            "backlog" | "ready" | "in-development" | "implemented"
                        ),
                        "in-development" => p.field("state") == "ready",
                        "implemented" => p.field("state") == "in-development",
                        _ => false,
                    };
                    if !allowed {
                        return Err(err("state-transition"));
                    }
                    if state != "backlog" {
                        if e.body["assignment"].is_null() {
                            if self
                                .selected(e, "assignment", e.issue())?
                                .is_some_and(|r| r.tag("t") == "assignment")
                            {
                                return Err(err("assignment-head"));
                            }
                        } else {
                            let bound = self.get(e.field("assignment"))?;
                            if bound.tag("t") != "assignment" {
                                return Err(err("assignment-head"));
                            }
                            self.current_binding(e, bound, "assignment-head")?;
                            let policy = self.get(e.tag("policy"))?;
                            let mut operation = Some(bound);
                            let mut seen = BTreeSet::new();
                            while let Some(op) = operation {
                                if !seen.insert(op.id()) || !self.assignment_authority(op, policy) {
                                    return Err(err("assignment-policy"));
                                }
                                operation = self.records.get(op.previous());
                            }
                        }
                    }
                    if state == "ready" {
                        let update = self.get(e.field("update"))?;
                        self.current_binding(e, update, "update-head")?;
                        let fields = self.fields_from(e.issue(), Some(update));
                        if a(&fields["acceptance_criteria"]).is_empty()
                            || fields["platform"].is_null()
                        {
                            return Err(err("ready-fields"));
                        }
                        let verdicts = self.before(e, "member-verdict");
                        if verdicts
                            .iter()
                            .any(|v| v.issue() == e.issue() && v.field("verdict") == "accepted")
                        {
                            return Err(err("resolved"));
                        }
                        if p.field("state") == "implemented" {
                            let rework = self.records.get(e.field("rework")).is_some_and(|v| {
                                v.typ() == "member-verdict"
                                    && v.issue() == e.issue()
                                    && v.field("verdict") == "rejected"
                                    && self.historical(v.id()).is_ok()
                            });
                            let terminal =
                                self.records
                                    .get(e.field("terminal_set"))
                                    .is_some_and(|set| {
                                        let valid = self.known_before(e);
                                        matches!(
                                            self.set_status(set.id(), e.time(), &valid),
                                            "failed" | "aborted"
                                        ) && a(&set.body["members"])
                                            .iter()
                                            .any(|m| s(&m["issue"]) == e.issue())
                                    });
                            if !rework && !terminal {
                                return Err(err("rework-required"));
                            }
                        }
                    }
                    if matches!(state, "in-development" | "implemented")
                        && (e.body["stream"] != p.body["stream"]
                            || e.body["assignment"] != p.body["assignment"]
                            || e.body["assignment"].is_null())
                    {
                        return Err(err("writer-binding"));
                    }
                }
            }
            "release-set" if e.field("action") == "freeze" => {
                if previous.is_some() {
                    return Err(err("wrong-previous"));
                }
                let valid = self.known_before(e);
                for m in a(&e.body["members"]) {
                    let issue = s(&m["issue"]);
                    self.current_binding(e, self.get(s(&m["implemented"]))?, "implemented-head")?;
                    if !self.is_executable_leaf(issue, &valid) {
                        return Err(err("non-leaf"));
                    }
                }
                if self.before(e, "release-set").iter().any(|s| {
                    s.field("pipeline") == e.field("pipeline")
                        && s.field("release") == e.field("release")
                }) {
                    return Err(err("release-label"));
                }
            }
            "release-set" => {
                if previous.is_none() {
                    return Err(err("wrong-previous"));
                }
                for req in self
                    .before(e, "build-request")
                    .iter()
                    .filter(|r| r.field("set") == e.field("set"))
                {
                    if !self.run_terminal_before(e, req.id()) {
                        return Err(err("build-lock"));
                    }
                }
            }
            "build-request" => {
                let valid = self.known_before(e);
                if self.terminal_at(e.field("set"), e.time(), &valid) {
                    return Err(err("set-terminal"));
                }
                if let Some(p) = previous {
                    if !self.run_terminal_before(e, p.id()) {
                        return Err(err("build-lock"));
                    }
                }
                if n(&e.body["attempt"]) == 1 {
                    if !e.body["retry_of"].is_null() {
                        return Err(err("retry"));
                    }
                } else {
                    let run = self.get(e.field("retry_of"))?;
                    let req = self.get(run.field("request"))?;
                    if !matches!(run.field("result"), "failure" | "cancelled")
                        || e.previous() != req.id()
                        || req.field("set") != e.field("set")
                        || n(&e.body["attempt"]) != n(&req.body["attempt"]) + 1
                    {
                        return Err(err("retry"));
                    }
                }
            }
            "build-run" => {
                if let Some(p) = previous {
                    if matches!(p.field("result"), "success" | "failure" | "cancelled")
                        || e.field("result") == "queued"
                        || e.field("result") == p.field("result")
                    {
                        return Err(err("run-transition"));
                    }
                    for key in ["request", "provider", "run_id", "attempt", "url", "head"] {
                        if p.body[key] != e.body[key] {
                            return Err(err("run-identity"));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn closed_issue(&self, issue: &str, valid: &[&Record]) -> bool {
        valid.iter().any(|r| {
            r.issue() == issue
                && (r.typ() == "member-verdict" && r.field("verdict") == "accepted"
                    || r.typ() == "triage-action"
                        && matches!(r.field("action"), "duplicate" | "decline"))
        })
    }
    /// An executable leaf per NIP-BW.md: "a non-closed, non-resolved issue with
    /// no active child, no duplicate-of edge and no unresolved blocker." Shared
    /// by the freeze-time membership check and the read-side projection, so a
    /// `parent-of`/`blocks`/`duplicate-of` conflict is judged identically in
    /// both places — the UI never re-derives this on its own.
    pub fn is_executable_leaf(&self, issue: &str, valid: &[&'a Record]) -> bool {
        let edges = self.relations(valid);
        !(self.closed_issue(issue, valid)
            || edges.iter().any(|r| {
                (r.issue() == issue
                    && (r.field("relation") == "duplicate-of"
                        || r.field("relation") == "parent-of"
                            && !self.closed_issue(r.field("target"), valid)))
                    || (r.field("target") == issue
                        && r.field("relation") == "blocks"
                        && !valid.iter().any(|v| {
                            v.typ() == "member-verdict"
                                && v.issue() == r.issue()
                                && v.field("verdict") == "accepted"
                        }))
            }))
    }
    fn run_terminal_before(&self, e: &Record, request: &str) -> bool {
        self.before(e, "build-run").iter().any(|r| {
            r.field("request") == request
                && matches!(r.field("result"), "success" | "failure" | "cancelled")
        })
    }
    pub fn known_before(&self, e: &Record) -> Vec<&'a Record> {
        self.records
            .values()
            .filter(|r| {
                r.id() != e.id()
                    && r.time() <= e.time()
                    && !self.depends_on(r, e.id())
                    && self.historical(r.id()).is_ok()
            })
            .collect()
    }
    pub fn relations(&self, valid: &[&'a Record]) -> Vec<&'a Record> {
        valid
            .iter()
            .filter(|r| {
                r.typ() == "issue-relation"
                    && r.field("operation") == "add"
                    && !valid.iter().any(|s| s.previous() == r.id())
            })
            .copied()
            .collect()
    }
}
