# GitHub treats success, skipped, and neutral as successful conclusions for
# required checks. Evaluate the newest run so a stale pass cannot mask a rerun.
[
  .[].check_runs[]
  | select(.name == $name)
]
| sort_by(.started_at // .created_at // "")
| last
| .status == "completed"
  and (
    .conclusion == "success"
    or .conclusion == "skipped"
    or .conclusion == "neutral"
  )
