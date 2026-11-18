{
  predicate_version: 1,
  eligible: true,
  source: {
    repository: $source_repository,
    ref: $source_ref,
    sha: $source_sha
  },
  build: {
    workflow: $build_workflow,
    run_id: $build_run_id,
    run_attempt: $build_run_attempt,
    run_url: $build_run_url
  },
  qualification: {
    workflow: $qualification_workflow,
    run_id: $qualification_run_id,
    run_attempt: $qualification_run_attempt,
    run_url: $qualification_run_url,
    conclusion: "success"
  },
  helm_chart: {
    name: "buzz",
    compatible_version: $chart_version
  }
}
