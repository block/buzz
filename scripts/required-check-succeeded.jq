# REST check runs do not expose per-attempt creation time. The endpoint is
# intentionally queried with filter=latest; select the highest immutable run ID
# for the trusted producer and require that returned attempt to have completed
# successfully by merge. Any ordinary post-merge rerun therefore fails closed
# and needs operator inspection. DCO alone has a bounded five-minute exception.
[
  .[].check_runs[]
  | select(.name == $name and .app.id == $integration_id)
]
| sort_by(.id)
| last
| select((.completed_at // null) != null)
| select(
    (.completed_at | fromdateiso8601)
    <= (
      ($merged_at | fromdateiso8601)
      + (if $name == "DCO Check" then 300 else 0 end)
    )
  )
| .status == "completed"
  and (
    .conclusion == "success"
    or .conclusion == "skipped"
    or .conclusion == "neutral"
  )
