// Model of the parts of Buzz's Git-on-object-storage push path that the TLA+
// model (docs/spec/GitOnObjectStore.tla) leaves out: the serving-write lease
// around the pointer CAS and the timestamps of the kind:30618 events. It also
// states the publish decision over what clients are told, which complements
// the refinement statement of the TLA+ model.
//
// - info/refs and the receive-pack POST are separate requests; each hydrates
//   from whatever pointer is current when it runs (transport.rs).
// - receive-pack applies each ref update independently (non-atomic push); a
//   pre-receive hook decline rejects all refs. The per-ref outcome is written
//   to the client only if it negotiated `report-status`, and a request may
//   carry no commands at all.
// - finalize_push decides whether to publish (`push_disposition`), then CASes
//   the manifest pointer inside a serving-write lease that can be lost around
//   the CAS (buzz-deletion).
// - after the CAS, the relay emits a kind:30618 ref-state event; clients read
//   the newest one, ties broken by lowest id
//   (desktop/src/features/projects/hooks.ts, buzz-db ORDER BY created_at DESC,
//   id ASC).
//
// Invariants are stated over what clients observe, not over protocol
// internals. PUBLISH_RULE selects the implemented publish decision or one of
// the two rules the relay used earlier, and each FIX_* constant switches one
// defect to its fix. `fixed` models the code as it is; `current` models the
// code before any of the fixes.
//
// Check with Quint (https://quint-lang.org):
//   quint run docs/spec/GitPushPath.qnt --main fixed --invariant safety \
//     --max-steps 14 --max-samples 2000000 --seed 0x1
//   quint verify docs/spec/GitPushPath.qnt --main fixed --invariant safety \
//     --max-steps 8
// The same commands with --main current, partialBugOnly, reportRuleOnly,
// leaseBugOnly, or timestampBugOnly report a counterexample.
//
// The initial manifest carries a stamp. The announce-time seed and manifests
// written before `published_at` existed carry none; the argument for them
// rests on an assumption about clocks that is stated in
// docs/git-on-object-storage.md and is not modeled here.
module buzzgit {
  // "refs":   publish when receive-pack changed the workspace refs (the
  //           implemented rule, `push_disposition`).
  // "report": publish when the status report has an `ok` line or no `ng`
  //           line (the rule before it).
  // "no_ng":  publish when the status report has no `ng` line (the original
  //           rule).
  const PUBLISH_RULE: str
  const FIX_LEASE: bool
  const FIX_TIMESTAMPS: bool

  // ---- Domain ----------------------------------------------------------

  type Refs = str -> int // ref name -> commit; 0 means absent

  type Push = {
    phase: str,          // idle | advertised | received | committed | done
    adv: Refs,           // refs the client saw in info/refs
    targets: Set[str],   // refs this push updates (each to commit = pusher id)
    base: int,           // manifest version receive-pack hydrated from
    ws: Refs,            // workspace refs after receive-pack
    report: str -> str,  // what receive-pack did with each command: ok | ng
    status: bool,        // did the client negotiate report-status?
    told: str -> str,    // what the client printed: ok | ng | fail
    stamp: int,          // created_at of the 30618 this push emits
  }

  type Manifest = { refs: Refs, stamp: int }
  type Event = { createdAt: int, id: int, refs: Refs }

  pure val REF_LIST = ["main", "feature", "feature/x"]
  pure val REFS = Set("main", "feature", "feature/x")
  pure val PUSHERS = Set(1, 2)
  pure val EVENT_IDS = 1.to(5)
  pure val MAX_TIME = 2

  pure val INITIAL_REFS: Refs = Map("main" -> 9, "feature" -> 9, "feature/x" -> 0)

  pure val IDLE: Push = {
    phase: "idle",
    adv: INITIAL_REFS,
    targets: Set(),
    base: 0,
    ws: INITIAL_REFS,
    report: Map(),
    status: true,
    told: Map(),
    stamp: 0,
  }

  // git refuses a ref whose name is a directory/file prefix of a live ref.
  pure def dfConflict(ws: Refs, r: str): bool =
    (r == "feature/x" and ws.get("feature") != 0) or
    (r == "feature" and ws.get("feature/x") != 0)

  // Non-atomic receive-pack: process commands in order; each applies only if
  // the workspace still holds the client's old value and the name is free.
  pure def receivePack(p: int, adv: Refs, targets: Set[str], ws0: Refs, hookDeny: bool)
      : { ws: Refs, report: str -> str } = {
    if (hookDeny)
      { ws: ws0, report: targets.mapBy(_ => "ng") }
    else
      REF_LIST.foldl({ ws: ws0, report: Map() }, (acc, r) =>
        if (not(targets.contains(r))) acc
        else if (acc.ws.get(r) == adv.get(r) and not(dfConflict(acc.ws, r)))
          { ws: acc.ws.set(r, p), report: acc.report.put(r, "ok") }
        else
          { ws: acc.ws, report: acc.report.put(r, "ng") })
  }

  pure def anyOk(report: str -> str): bool = report.keys().exists(r => report.get(r) == "ok")
  pure def anyNg(report: str -> str): bool = report.keys().exists(r => report.get(r) == "ng")

  // The status report as the relay and the client see it: git writes the
  // per-ref lines only for a client that negotiated report-status.
  pure def visible(pr: Push): str -> str = if (pr.status) pr.report else Map()

  // transport.rs. `hydrated` is the ref state the workspace was built from.
  pure def shouldPublish(pr: Push, hydrated: Refs): bool =
    if (PUBLISH_RULE == "refs") pr.ws != hydrated
    else if (PUBLISH_RULE == "report") anyOk(visible(pr)) or not(anyNg(visible(pr)))
    else not(anyNg(visible(pr)))

  pure def max(a: int, b: int): int = if (a > b) a else b

  // ---- State -------------------------------------------------------------

  var ptr: int                     // current manifest version (the pointer)
  var manifests: int -> Manifest   // published manifests by version
  var nextVersion: int
  var events: Set[Event]           // stored kind:30618 events
  var now: int                     // wall clock (seconds)
  var push: int -> Push
  var landed: Set[(int, str)]      // (pusher, ref) updates that reached the pointer

  // ---- Actions -----------------------------------------------------------

  action init = all {
    ptr' = 0,
    manifests' = Map(0 -> { refs: INITIAL_REFS, stamp: 0 }),
    nextVersion' = 1,
    events' = Set({ createdAt: 0, id: 0, refs: INITIAL_REFS }),
    now' = 0,
    push' = PUSHERS.mapBy(_ => IDLE),
    landed' = Set(),
  }

  // GET info/refs: the client learns the current refs and picks updates.
  action advertise(p: int): bool = all {
    push.get(p).phase == "idle",
    // A request may carry no commands.
    nondet targets = REFS.powerset().filter(s => s.size() <= 2).oneOf()
    push' = push.set(p, { ...push.get(p),
      phase: "advertised",
      adv: manifests.get(ptr).refs,
      targets: targets,
    }),
    ptr' = ptr, manifests' = manifests, nextVersion' = nextVersion,
    events' = events, now' = now, landed' = landed,
  }

  // POST git-receive-pack: hydrate from the *current* pointer, run hook and
  // per-ref updates in the ephemeral workspace.
  action receive(p: int): bool = all {
    push.get(p).phase == "advertised",
    nondet hookDeny = Set(true, false).oneOf()
    nondet status = Set(true, false).oneOf()
    val pr = push.get(p)
    val result = receivePack(p, pr.adv, pr.targets, manifests.get(ptr).refs, hookDeny)
    push' = push.set(p, { ...pr,
      phase: "received",
      base: ptr,
      ws: result.ws,
      report: result.report,
      status: status,
    }),
    ptr' = ptr, manifests' = manifests, nextVersion' = nextVersion,
    events' = events, now' = now, landed' = landed,
  }

  // finalize_push when it does not publish: either the rule found nothing to
  // publish, or the CAS lost the race (409, the client sees an HTTP error for
  // every ref).
  action finalizeNoPublish(p: int): bool = all {
    val pr = push.get(p)
    val publish = shouldPublish(pr, manifests.get(pr.base).refs)
    all {
      pr.phase == "received",
      not(publish) or ptr != pr.base,
      // Skipped publish: git's report-status goes back verbatim.
      // Lost CAS: 409, an HTTP error for every ref.
      push' = push.set(p, { ...pr,
        phase: "done",
        told: if (publish) pr.targets.mapBy(_ => "fail") else visible(pr),
      }),
    },
    ptr' = ptr, manifests' = manifests, nextVersion' = nextVersion,
    events' = events, now' = now, landed' = landed,
  }

  // finalize_push CAS. The serving-write lease may be lost after the pointer
  // PUT lands; `protect` then discards the result and returns 503.
  action commit(p: int): bool = all {
    val pr = push.get(p)
    all {
      pr.phase == "received",
      shouldPublish(pr, manifests.get(pr.base).refs),
      ptr == pr.base,
      nondet leaseLostAfterCas = Set(true, false).oneOf()
      nondet skew = Set(0, 1).oneOf() // per-instance clock offset
      val okRefs = pr.report.keys().filter(r => pr.report.get(r) == "ok")
      val parentStamp = manifests.get(ptr).stamp
      val stamp = if (FIX_TIMESTAMPS) max(now + skew, parentStamp + 1) else now + skew
      all {
        ptr' = nextVersion,
        manifests' = manifests.put(nextVersion, { refs: pr.ws, stamp: stamp }),
        nextVersion' = nextVersion + 1,
        landed' = landed.union(okRefs.map(r => (p, r))),
        // On lease loss: 503 "community write lease lost"; no 30618 is emitted.
        push' = push.set(p,
          if (leaseLostAfterCas and not(FIX_LEASE))
            { ...pr, phase: "done", told: pr.targets.mapBy(_ => "fail") }
          else
            { ...pr, phase: "committed", stamp: stamp }),
      },
    },
    events' = events, now' = now,
  }

  // Derived kind:30618 insert, then the success response.
  action emit(p: int): bool = all {
    val pr = push.get(p)
    val usedIds = events.map(e => e.id)
    all {
      pr.phase == "committed",
      nondet id = EVENT_IDS.exclude(usedIds).oneOf()
      events' = events.union(Set({ createdAt: pr.stamp, id: id, refs: pr.ws })),
      push' = push.set(p, { ...pr, phase: "done", told: visible(pr) }),
    },
    ptr' = ptr, manifests' = manifests, nextVersion' = nextVersion,
    now' = now, landed' = landed,
  }

  action tick = all {
    now < MAX_TIME,
    now' = now + 1,
    ptr' = ptr, manifests' = manifests, nextVersion' = nextVersion,
    events' = events, push' = push, landed' = landed,
  }

  action step = any {
    nondet p = PUSHERS.oneOf()
    any { advertise(p), receive(p), finalizeNoPublish(p), commit(p), emit(p) },
    tick,
  }

  // ---- Observations --------------------------------------------------------

  // The 30618 a reader picks: newest created_at, then lowest id.
  def latestEvent: Event =
    events.fold({ createdAt: -1, id: 0, refs: INITIAL_REFS }, (best, e) =>
      if (e.createdAt > best.createdAt or (e.createdAt == best.createdAt and e.id < best.id)) e
      else best)

  def quiescent: bool = PUSHERS.forall(p => push.get(p).phase == "done")

  // ---- Invariants ----------------------------------------------------------

  // A ref the client printed as pushed reached the published pointer.
  val noFalseSuccess: bool = PUSHERS.forall(p =>
    val t = push.get(p).told
    t.keys().forall(r => t.get(r) == "ok" implies landed.contains((p, r))))

  // A ref the client was told failed did not reach the published pointer.
  val noFalseFailure: bool = PUSHERS.forall(p =>
    val t = push.get(p).told
    t.keys().forall(r => t.get(r) == "fail" implies not(landed.contains((p, r)))))

  // Once pushes settle, the ref state readers see matches the pointer.
  val refStateMatchesPointer: bool =
    quiescent implies latestEvent.refs == manifests.get(ptr).refs

  // Every publication changes the refs. A manifest whose refs equal its
  // predecessor's records no push, yet its kind:30618 names the caller as the
  // pusher. Versions are allocated in pointer order, so the predecessor of
  // version v is v - 1.
  val publicationChangesRefs: bool = 1.to(nextVersion - 1).forall(v =>
    manifests.get(v).refs != manifests.get(v - 1).refs)

  val safety: bool = and {
    noFalseSuccess,
    noFalseFailure,
    refStateMatchesPointer,
    publicationChangesRefs,
  }
}

// The code before the fixes.
module current {
  import buzzgit(PUBLISH_RULE = "no_ng", FIX_LEASE = false, FIX_TIMESTAMPS = false).*
}

// The code as it is.
module fixed {
  import buzzgit(PUBLISH_RULE = "refs", FIX_LEASE = true, FIX_TIMESTAMPS = true).*
}

// One defect at a time, to attribute each counterexample.

// The original rule: violates noFalseSuccess (a partially accepted push is
// not published) and publicationChangesRefs.
module partialBugOnly {
  import buzzgit(PUBLISH_RULE = "no_ng", FIX_LEASE = true, FIX_TIMESTAMPS = true).*
}

// The rule that reads the status report: satisfies noFalseSuccess, violates
// publicationChangesRefs (a request with no commands, or a declined push from
// a client without report-status, is published).
module reportRuleOnly {
  import buzzgit(PUBLISH_RULE = "report", FIX_LEASE = true, FIX_TIMESTAMPS = true).*
}

module leaseBugOnly {
  import buzzgit(PUBLISH_RULE = "refs", FIX_LEASE = false, FIX_TIMESTAMPS = true).*
}

module timestampBugOnly {
  import buzzgit(PUBLISH_RULE = "refs", FIX_LEASE = true, FIX_TIMESTAMPS = false).*
}
