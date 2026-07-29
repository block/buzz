def tag_values($name):
  [.tags[]? | select(.[0] == $name and .[1] != null) | .[1]];

def tag_value($name):
  tag_values($name)[0];

def body:
  try (.content | fromjson) catch {};

def declaration_role:
  (tag_value("d") // "") | split("/")[0];

def declaration_stream:
  (tag_value("d") // "") as $d
  | $d | ltrimstr(($d | split("/")[0]) + "/");

def active:
  (body.status // "active") == "active";

def effective_heads($events):
  $events
  | map(select(tag_value("d") != null))
  | group_by([.pubkey, .kind, tag_value("d")])
  | map(
      (map(.created_at) | max) as $created_at
      | map(select(.created_at == $created_at))
      | min_by(.id)
    );

def short_key($key):
  if ($key | length) > 12 then "\($key[0:12])..." else $key end;

def stream_status($stream; $declarations):
  ([$declarations[]
    | select(
        declaration_role == "export"
        and declaration_stream == $stream
        and active
      )][0]) as $export
  | if $export == null then
      "no_export"
    else
      (($export | tag_values("p")) as $offered
       | [$declarations[]
          | select(
              declaration_role == "admit"
              and declaration_stream == $stream
              and active
            )
          | . as $admit
          | select($offered | index($admit.pubkey) != null)]) as $admits
      | if ($admits | length) == 0 then
          "open"
        elif any($admits[]; tag_value("e") == $export.id) then
          "matched"
        elif any($admits[]; tag_value("e") == null) then
          "unpinned"
        else
          "drift"
        end
    end;

def selection_label($event):
  (body.selection // {}) as $selection
  | if $selection.mirror == true then
      "mirror"
    elif $selection.from_source != null then
      "from_source:\($selection.from_source)"
    elif $selection.filter != null then
      "filter"
    else
      "unspecified"
    end;

def namespace_paths($stream):
  ($stream | split("/")) as $parts
  | [range(1; $parts | length) as $depth
     | $parts[0:$depth] | join("/")];

def node_id($type; $value):
  "\($type):\($value)";

. as $input
| if type != "array" then
    error("buzz-ctx graph input must be a JSON event array")
  else
    .
  end
| effective_heads($input) as $heads
| ([$input[] | select(tag_value("d") == null)] + $heads) as $events
| [$heads[] | select(.kind == 30700 and tag_value("d") != null)] as $declarations
| ([$declarations[]
    | select(
        declaration_role == "export"
        or declaration_role == "admit"
        or declaration_role == "read"
      )
    | declaration_stream]
   | unique
   | sort) as $stream_ids
| ([$events[] | tag_values("h")[]]
   + [$heads[]
      | select(.kind >= 39000 and .kind <= 39003)
      | tag_value("d")
      | select(. != null)]
   + [$declarations[]
      | select(declaration_role == "export")
      | (body.selection.filter // [])[]
      | ((."#h" // []) + (."#d" // []))[]])
  | unique
  | sort as $context_ids
| ([$events[] | tag_values("x")[]] | unique | sort) as $artifact_ids
| ([$events[].pubkey] + [$events[] | tag_values("p")[]])
  | unique
  | sort as $identity_ids
| ([$declarations[] | tag_value("n") | select(. != null)])
  | unique
  | sort as $node_labels
| ([$stream_ids[] | namespace_paths(.)[]] | unique | sort) as $namespaces
| (
    [$identity_ids[] as $identity
      | {
          id: node_id("identity"; $identity),
          type: "identity",
          label: short_key($identity),
          pubkey: $identity,
          authority: "cryptographic"
        }]
    + [$node_labels[] as $node
       | {
           id: node_id("node"; $node),
           type: "node",
           label: $node
         }]
    + [$namespaces[] as $namespace
       | {
           id: node_id("namespace"; $namespace),
           type: "namespace",
           label: $namespace
         }]
    + [$stream_ids[] as $stream
       | {
           id: node_id("stream"; $stream),
           type: "stream",
           label: $stream,
           status: stream_status($stream; $declarations)
         }]
    + [$context_ids[] as $context
       | ([$heads[]
           | select(.kind == 39000 and tag_value("d") == $context)][0]) as $metadata
       | {
           id: node_id("context"; $context),
           type: "context",
           label: ($metadata | tag_value("name")) // $context,
           context_id: $context,
           visibility: (($metadata | tag_value("visibility")) // "unspecified")
         }]
    + [$heads[]
       | select(.kind != 30700 and (.kind < 39000 or .kind > 39003))
       | (tag_value("d")) as $d
       | {
           id: node_id("head"; "\(.pubkey):\(.kind):\($d)"),
           type: "head",
           label: (
             tag_value("title")
             // tag_value("name")
             // (if .kind == 30174 then "encrypted:\($d[0:12])..." else $d end)
           ),
           coordinate: "\(.kind):\(.pubkey):\($d)",
           kind: .kind,
           event_id: .id
         }]
    + [$artifact_ids[] as $artifact
       | {
           id: node_id("artifact"; $artifact),
           type: "artifact",
           label: "\($artifact[0:12])...",
           sha256: $artifact
         }]
  )
  | unique_by(.id)
  | sort_by(.type, .label, .id) as $nodes
| (
    [$declarations[]
      | select(tag_value("n") != null)
      | {
          from: node_id("identity"; .pubkey),
          to: node_id("node"; tag_value("n")),
          type: "controls",
          event_id: .id
        }]
    + [$stream_ids[] as $stream
       | namespace_paths($stream) as $paths
       | [range(0; $paths | length) as $index
          | if $index == 0 then
              empty
            else
              {
                from: node_id("namespace"; $paths[$index - 1]),
                to: node_id("namespace"; $paths[$index]),
                type: "contains"
              }
            end][]
      ]
    + [$stream_ids[] as $stream
       | namespace_paths($stream) as $paths
       | if ($paths | length) == 0 then
           empty
         else
           {
             from: node_id("namespace"; $paths[-1]),
             to: node_id("stream"; $stream),
             type: "contains"
           }
         end]
    + [$declarations[]
       | select(declaration_role == "export")
       | . as $event
       | {
           from: (
             if tag_value("n") == null then
               node_id("identity"; .pubkey)
             else
               node_id("node"; tag_value("n"))
             end
           ),
           to: node_id("stream"; declaration_stream),
           type: "exports",
           status: (body.status // "active"),
           selection: selection_label($event),
           event_id: .id
         }]
    + [$declarations[]
       | select(declaration_role == "export")
       | . as $event
       | tag_values("p")[]
       | {
           from: node_id("stream"; ($event | declaration_stream)),
           to: node_id("identity"; .),
           type: "offered_to",
           event_id: $event.id
         }]
    + [$declarations[]
       | select(declaration_role == "admit")
       | . as $event
       | {
           from: node_id("identity"; .pubkey),
           to: node_id("stream"; declaration_stream),
           type: "admits",
           status: (body.status // "active"),
           pin: tag_value("e"),
           event_id: .id
         }]
    + [$declarations[]
       | select(declaration_role == "read")
       | . as $event
       | tag_values("p")[]
       | {
           from: node_id("stream"; ($event | declaration_stream)),
           to: node_id("identity"; .),
           type: "readable_by",
           status: ($event | body.status // "active"),
           pin: ($event | tag_value("e")),
           event_id: $event.id
         }]
    + [$declarations[]
       | select(declaration_role == "steward")
       | . as $event
       | tag_values("p")[]
       | {
           from: node_id("identity"; .),
           to: node_id("node"; ($event | tag_value("n"))),
           type: "stewards",
           status: ($event | body.status // "active"),
           event_id: $event.id
         }]
    + [$declarations[]
       | select(declaration_role == "export")
       | . as $event
       | (body.selection.from_source // empty)
       | {
           from: node_id("stream"; ($event | declaration_stream)),
           to: node_id("stream"; .),
           type: "selects_source",
           event_id: $event.id
         }]
    + [$declarations[]
       | select(declaration_role == "export")
       | . as $event
       | (body.selection.filter // [])[]
       | ((."#h" // []) + (."#d" // []))[]
       | {
           from: node_id("stream"; ($event | declaration_stream)),
           to: node_id("context"; .),
           type: "carries",
           event_id: $event.id
         }]
    + [$heads[]
       | select(.kind == 39000)
       | {
           from: node_id("identity"; .pubkey),
           to: node_id("context"; tag_value("d")),
           type: "owns",
           event_id: .id
         }]
    + [$heads[]
       | select(.kind == 39002)
       | . as $event
       | tag_values("p")[]
       | {
           from: node_id("identity"; .),
           to: node_id("context"; ($event | tag_value("d"))),
           type: "member_of",
           event_id: $event.id
         }]
    + [$events[]
       | select(tag_values("h") | length > 0)
       | . as $event
       | tag_values("h")[]
       | {
           from: node_id("identity"; $event.pubkey),
           to: node_id("context"; .),
           type: "contributes_to"
         }]
    + [$heads[]
       | select(.kind != 30700 and (.kind < 39000 or .kind > 39003))
       | . as $event
       | {
           from: node_id("identity"; .pubkey),
           to: node_id("head"; "\(.pubkey):\(.kind):\(tag_value("d"))"),
           type: "owns",
           event_id: .id
         }]
    + [$events[]
       | select(tag_values("x") | length > 0)
       | . as $event
       | tag_values("x")[]
       | {
           from: (
             if ($event | tag_value("h")) != null then
               node_id("context"; ($event | tag_value("h")))
             elif ($event | tag_value("d")) != null then
               node_id("head"; "\($event.pubkey):\($event.kind):\($event | tag_value("d"))")
             else
               node_id("identity"; $event.pubkey)
             end
           ),
           to: node_id("artifact"; .),
           type: "references"
         }]
  )
  | unique_by([.from, .to, .type, (.event_id // "")])
  | sort_by(.from, .to, .type, (.event_id // "")) as $edges
| (
    [$stream_ids[] as $stream
     | stream_status($stream; $declarations) as $status
     | select($status != "matched")
     | {
         code: ("agreement_" + $status),
         severity: (if $status == "open" then "info" else "warning" end),
         stream: $stream,
         message: "stream \($stream) agreement is \($status)"
       }]
    + [$declarations[]
       | select(tag_value("n") == null)
       | {
           code: "unscoped_declaration",
           severity: "warning",
           stream: declaration_stream,
           event_id: .id,
           message: "\(tag_value("d")) has no node scope"
         }]
    + [$declarations[]
       | select(
           declaration_role == "export"
           and active
           and body.selection.mirror == true
         )
       | {
           code: "whole_journal_mirror",
           severity: "info",
           stream: declaration_stream,
           event_id: .id,
           message: "stream \(declaration_stream) exports a whole-journal mirror"
         }]
    + [$declarations[]
       | select(declaration_role == "read" and active)
       | . as $read
       | ([$declarations[]
           | select(
               declaration_role == "export"
               and declaration_stream == ($read | declaration_stream)
               and active
             )][0]) as $export
       | select($export != null and ($read | tag_value("e")) != $export.id)
       | {
           code: "read_pin_drift",
           severity: "warning",
           stream: ($read | declaration_stream),
           event_id: $read.id,
           message: "read grant for \($read | declaration_stream) does not pin the effective export"
         }]
  )
  | unique_by([.code, (.stream // ""), (.event_id // "")])
  | sort_by(.severity, .code, (.stream // ""), (.event_id // "")) as $warnings
| {
    schema: "buzz-context-graph/v1",
    source_event_count: ($input | length),
    effective_event_count: ($events | length),
    nodes: $nodes,
    edges: $edges,
    effective_heads: (
      [$heads[]
       | {
           coordinate: "\(.kind):\(.pubkey):\(tag_value("d"))",
           event_id: .id,
           pubkey: .pubkey,
           kind: .kind,
           d: tag_value("d"),
           created_at: .created_at
         }]
      | sort_by(.coordinate)
    ),
    warnings: $warnings
  }
