#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RENDERER="$ROOT/scripts/buzz-ctx-graph"
FIXTURE="$ROOT/scripts/testdata/buzz-ctx-graph-events.json"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

"$RENDERER" --format json < "$FIXTURE" > "$TMP/graph.json"
"$RENDERER" --format tree < "$FIXTURE" > "$TMP/graph.tree"
"$RENDERER" --format dot < "$FIXTURE" > "$TMP/graph.dot"
"$RENDERER" --format mermaid < "$FIXTURE" > "$TMP/graph.mermaid"

jq -e '
  . as $graph
  | .schema == "buzz-context-graph/v1"
  and any(.nodes[]; .type == "stream"
    and .label == "shared/buzz-evolution-v1"
    and .status == "matched")
  and any(.nodes[]; .type == "namespace" and .label == "shared")
  and any(.nodes[]; .type == "context" and .label == "Buzz Evolution")
  and any(.nodes[]; .type == "artifact"
    and .sha256 == "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd")
  and any(.edges[]; .type == "carries"
    and .to == "context:11111111-2222-4333-8444-555555555555")
  and any(.edges[]; .type == "member_of"
    and .from == "identity:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
  and any(.effective_heads[]; .event_id == "export-new")
  and any(.effective_heads[]; .event_id == "a-metadata")
  and all(.effective_heads[]; .event_id != "export-old" and .event_id != "b-metadata")
  and all($graph.edges[];
    . as $edge
    | any($graph.nodes[]; .id == $edge.from)
      and any($graph.nodes[]; .id == $edge.to))
  and (.warnings | length) == 0
' "$TMP/graph.json" >/dev/null

grep -Fq 'buzz-evolution-v1 [matched]' "$TMP/graph.tree"
grep -Fq 'Buzz Evolution [private]' "$TMP/graph.tree"
grep -Fq 'digraph buzz_context {' "$TMP/graph.dot"
grep -Fq 'label="carries"' "$TMP/graph.dot"
grep -Fq 'flowchart LR' "$TMP/graph.mermaid"
grep -Fq -- '-->|carries|' "$TMP/graph.mermaid"

jq 'map(if .id == "admit-new" then
  .tags = (.tags | map(if .[0] == "e" then ["e", "stale-export"] else . end))
  else . end)' "$FIXTURE" |
  "$RENDERER" --format json > "$TMP/drift.json"
jq -e '
  any(.nodes[]; .type == "stream"
    and .label == "shared/buzz-evolution-v1"
    and .status == "drift")
  and any(.warnings[]; .code == "agreement_drift")
' "$TMP/drift.json" >/dev/null

if "$RENDERER" --format invalid < "$FIXTURE" >/dev/null 2>&1; then
  echo "invalid graph format unexpectedly succeeded" >&2
  exit 1
fi

echo "buzz-ctx graph conformance: ok"
