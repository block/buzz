[
  .workflow_runs[]
  | select(.head_sha == $source_sha)
  | select(.event == "push")
  | select(.head_branch == "main" or .head_branch == "release")
]
| sort_by([.id, .run_attempt])
| last // empty
| select(.conclusion == "success")
