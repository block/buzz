------------------------ MODULE MCGitOnObjectStore ------------------------
(***************************************************************************)
(* Model-checking instance of GitOnObjectStore.                            *)
(*                                                                         *)
(* The object graph has two objects and o2 refers to o1, so a delta pack,  *)
(* a pack that is missing an ancestor, and compaction all occur.           *)
(***************************************************************************)
EXTENDS GitOnObjectStore, TLC

CONSTANTS o1, o2

MCObjParents == [o \in Oids |-> IF o = o2 THEN {o1} \cap Oids ELSE {}]
ClientSymmetry == Permutations(Clients)

(***************************************************************************)
(* State identification.  The stores only grow, and most of what they hold *)
(* is never read again: a manifest staged by a push that lost its race, or *)
(* one that has since been superseded.  TLC is told to identify two states *)
(* that differ only in store content that is not read.                     *)
(*                                                                         *)
(* Why this loses nothing for the protocol as specified.  The stores are   *)
(* read in three places: the hydration guard of Begin, and the invariants  *)
(* Fence and ReaderComplete.  Each reads only the manifests in Read below  *)
(* (the one the pointer names and the one the reader resolved) and the     *)
(* packs those manifests list.  Another manifest joins that set only when  *)
(* a pointer write installs it, and the writing process has stored it and  *)
(* its packs before that step (Upload, SetBegin, Seed).  So two states     *)
(* with equal views have the same enabled steps and the same invariant     *)
(* values, and their successors again have equal views.                    *)
(*                                                                         *)
(* Under a seeded fault that argument fails.  With the fault "gc", a pack  *)
(* staged by a push in progress can be deleted; the view hides the         *)
(* deletion, and TLC then misses the later pointer write that names the    *)
(* missing pack.  A view can only hide a violation, never invent one,      *)
(* because every state TLC explores is reachable.  check.sh therefore runs *)
(* every seeded-fault configuration without the view, and its check        *)
(* "no-view" repeats the two-client check of the protocol as specified     *)
(* without the view.                                                       *)
(***************************************************************************)
ReadManifests ==
    (IF pointer = Absent THEN {} ELSE {pointer.body})
        \cup (IF rd = NoRead THEN {} ELSE {rd})

LiveView ==
    <<pointer, mstore \cap ReadManifests,
      pstore \cap UNION {m.packs : m \in ReadManifests},
      writes, pc, base, call, stat, staged, lin, resp,
      spc, sbase, starget, rd, credit>>
=============================================================================
