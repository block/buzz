-------------------------- MODULE MCGitGovernance --------------------------
(***************************************************************************)
(* Model-checking instances of GitGovernance.                              *)
(*                                                                         *)
(* One cast and one history serve every instance; Scenario picks which     *)
(* governance the community starts with and which moves each key may make. *)
(* Each scenario is small on purpose: it gives TLC every interleaving of a *)
(* handful of moves that the promises are about, and docs/spec/check.sh    *)
(* runs each one with the design and with the seeded errors it exercises.  *)
(*                                                                         *)
(* The cast:                                                               *)
(*   a   community admin, admin of the repo channel ch1                    *)
(*   b   member of ch1, who pushes; admin of ch1 in "governance"           *)
(*   c   member of ch1, who owns the code at c0 and lands changes          *)
(*   d   member of ch1, who reviews; c1 names d as code owner              *)
(*   m   community member outside ch1                                      *)
(*   g   an agent; b is its owner unless the scenario leaves it unowned    *)
(*   ci  the key trusted to report check "ci"                              *)
(*   t2  the key trusted to report check "lint" only                       *)
(*                                                                         *)
(* "fence" has two pushes of r1's main, one by g under a grant that can   *)
(* expire, and an admin who can ban g or change the floor and r2's rules   *)
(* while one of them waits to publish.                                     *)
(*                                                                         *)
(* The history:   c0 ── c1 ── c2        c0 ── s1 ── s2   (s1 has a secret) *)
(*                  └── x1                                                 *)
(***************************************************************************)
EXTENDS GitGovernance, TLC

CONSTANTS a, b, c, d, m, g, ci, t2, ch1, ch2, r1, r2, main, feat, v1,
          c0, c1, c2, x1, s1, s2, Scenario, MaxVersion, MaxDecisions

MCParent == [x \in Commits |->
               CASE x = c1 -> c0 [] x = c2 -> c1 [] x = x1 -> c0
                 [] x = s1 -> c0 [] x = s2 -> s1 [] OTHER -> NoCommit]

MCCodeOwners == [x \in Commits \cup {NoCommit} |->
                   CASE x = c0 -> {c} [] x = c1 -> {d} [] OTHER -> {}]

Rule(req, f, scope, n, chk) == [req |-> req, ref |-> f, scope |-> scope, n |-> n, check |-> chk]

\* Governance bodies, by name.
MCContent ==
    [x \in {"none", "review", "fresh", "check", "owners", "rules", "secrets",
            "trust", "pay", "repo", "repo2", "main", "feat", "bind1", "bind2"} |->
       CASE x = "none"    -> {}
         [] x = "review"  -> {Rule("review", main, "all", 1, None)}
         [] x = "fresh"   -> {Rule("review", main, "all", 1, None),
                              Rule("fresh-target", main, "all", 0, None)}
         [] x = "check"   -> {Rule("check", main, "all", 0, "ci")}
         [] x = "owners"  -> {Rule("codeowners", main, "all", 0, None)}
         [] x = "rules"   -> {Rule("require-patch", main, "all", 0, None),
                              Rule("no-force", v1, "pay", 0, None),
                              Rule("no-delete", v1, "pay", 0, None)}
         [] x = "secrets" -> {Rule("admin-push", main, "all", 0, None)}
         [] x = "trust"   -> {<<"ci", ci>>, <<"lint", t2>>}
         [] x = "pay"     -> {"pay"}
         [] x = "repo"    -> {Rule("no-force", main, "repo", 0, None)}
         [] x = "repo2"   -> {Rule("no-delete", main, "repo", 0, None)}
         [] x = "main"    -> main
         [] x = "feat"    -> feat
         [] x = "bind1"   -> ch1
         [] x = "bind2"   -> ch2]

S == Scenario

StartFloor ==
    CASE S \in {"review", "fresh", "check", "owners", "rules", "secrets"} -> S
      [] S \in {"staleok", "repush"} -> "review"
      [] OTHER -> "none"

MCInit ==
    [commRole |-> [k \in Keys |-> IF k = a THEN "admin" ELSE "member"],
     chanRole |-> [ch \in Channels |-> [k \in Keys |->
                     CASE ch = ch1 /\ k = a -> "admin"
                       [] ch = ch1 /\ k = b -> IF S = "governance" THEN "admin" ELSE "member"
                       [] ch = ch1 /\ k \in {c, d, ci, t2} -> "member"
                       [] ch = ch1 /\ k = g -> CASE S = "authority" -> "guest"
                                                   [] S = "roles" -> "admin"
                                                   [] OTHER -> "member"
                       [] ch = ch2 /\ k = a -> "admin"
                       [] OTHER -> "none"]],
     owner    |-> [x \in Agents |-> IF S = "authority" THEN None ELSE b],
     grants   |-> IF S \in {"review", "roles", "fence"} THEN {<<g, r1>>} ELSE {},
     gov      |-> [x \in DocIds |->
                     CASE x[1] = "floor"   -> StartFloor
                       [] x[1] = "trusted" -> "trust"
                       [] x[1] = "labels"  -> IF S = "rules" /\ x[2] = r1 THEN "pay" ELSE "none"
                       [] x[1] = "rules"   -> IF S = "rules" /\ x[2] = r1 THEN "repo2" ELSE "none"
                       [] x[1] = "bind"    -> "bind1"
                       [] x[1] = "head"    -> "main"],
     refs     |-> [r \in Repos |-> [f \in Refs |->
                     IF r = r2 /\ f = main THEN c1 ELSE c0]],
     visible  |-> [r \in Repos |-> IF r = r2 \/ S = "repush" THEN {c0, c1} ELSE {c0}],
     pushed   |-> IF S = "repush" THEN {<<a, r1, c1>>} ELSE {}]

Push(k, cmds) == <<k, r1, cmds>>
One(f, x) == {<<f, x>>}

MCProposals ==
    CASE S = "authority" -> {Push(b, One(main, c1)), Push(g, One(main, c1)), Push(m, One(main, c1))}
      [] S = "review"    -> {Push(a, One(feat, c1)), Push(g, One(feat, c2)),
                             Push(c, One(main, c1)), Push(c, One(main, c2))}
      [] S \in {"fresh", "staleok"} ->
                            {Push(b, One(feat, c2)), Push(b, One(main, c1)), Push(b, One(main, c2))}
      [] S \in {"check", "owners"} -> {Push(b, One(feat, c1)), Push(b, One(main, c1))}
      [] S = "rules"     -> {Push(b, One(main, c1)), Push(a, One(v1, NoCommit))}
      [] S = "secrets"   -> {Push(b, One(main, s2)), Push(b, One(feat, s1)), Push(b, One(feat, s2)),
                             Push(b, {<<feat, c1>>, <<main, c1>>})}
      [] S = "reads"     -> {Push(b, One(feat, c1))}
      [] S = "repush"    -> {Push(b, One(feat, c1)), Push(c, One(main, c1))}
      [] S = "fence"     -> {Push(b, One(main, c1)), Push(g, One(main, x1))}
      [] OTHER -> {}

Approves(keys, f, x) == {<<k, r1, f, x>> : k \in keys}
MCReviews ==
    CASE S = "review" -> Approves({b, c, d, g, ci}, main, c1) \cup Approves({d}, feat, c2)
                         \cup Approves({b}, main, c2) \cup {<<d, r2, main, c1>>}
      [] S \in {"fresh", "staleok"} -> Approves({c}, main, c1) \cup Approves({c}, main, c2)
      [] S = "owners" -> Approves({c, d}, main, c1)
      [] S = "repush" -> Approves({b}, main, c1)
      [] OTHER -> {}

MCPosts ==
    IF S = "check"
    THEN {<<ci, "ci", r1, c1, "pass">>, <<ci, "ci", r1, c1, "fail">>,
          <<m, "ci", r1, c1, "pass">>, <<t2, "ci", r1, c1, "pass">>}
    ELSE {}

MCRoleChanges ==
    CASE S = "authority" -> {<<ch1, b, "none">>, <<ch1, g, "member">>}
      [] S = "review"    -> {<<ch1, d, "none">>}
      [] S = "reads"     -> {<<ch1, b, "none">>}
      [] S = "governance" -> {<<ch1, b, "member">>}
      [] S = "roles"     -> {<<ch1, b, "admin">>, <<ch1, d, "admin">>}
      [] OTHER -> {}

MCBans == CASE S = "authority" -> {<<a, b>>} [] S = "fence" -> {<<a, g>>} [] OTHER -> {}

MCRegistrations == IF S = "authority" THEN {<<g, b>>, <<g, m>>} ELSE {}
MCGrantChoices == IF S = "authority" THEN {<<g, r1>>} ELSE {}

MCReadChoices ==
    CASE S = "reads"   -> {<<k, r1, x>> : k \in {b, m}, x \in Doors}
      [] S = "secrets" -> {<<c, r1, "evidence">>}
      [] S = "fence"   -> {<<g, r1, "fetch">>}
      [] OTHER -> {}

MCGovChoices ==
    CASE S = "fence" -> {<<a, <<"floor", None>>, "review">>, <<a, <<"rules", r2>>, "repo">>}
      [] S = "governance" ->
         {<<a, <<"floor", None>>, "review">>, <<b, <<"floor", None>>, "review">>,
          <<b, <<"labels", r1>>, "pay">>, <<b, <<"rules", r1>>, "repo">>,
          <<b, <<"rules", r2>>, "repo">>, <<a, <<"rules", r2>>, "repo2">>,
          <<m, <<"head", r1>>, "feat">>, <<m, <<"bind", r1>>, "bind2">>,
          <<b, <<"bind", r1>>, "bind2">>, <<a, <<"bind", r1>>, "bind2">>}
      [] OTHER -> {}

MCTagChoices == IF S = "governance" THEN {<<b, r1, "pay">>} ELSE {}
MCArchives == IF S = "reads" THEN {r1} ELSE {}

\* RepoKey: the agent announced r1, so both shortcut errors are reachable;
\* in "governance" an outsider announced it.
MCRepoKey == [r \in Repos |-> IF S = "governance" THEN m ELSE g]

Bounded ==
    /\ Cardinality(receipts) <= MaxDecisions /\ Cardinality(audit) <= MaxDecisions
    /\ Cardinality(govlog) <= MaxVersion
=============================================================================
