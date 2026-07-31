def expected_default_pull_request_rule($ruleset_id):
  [.rule_evaluations[] | select(
    .rule_source.type == "ruleset" and
    .rule_source.id == $ruleset_id and
    .enforcement == "active" and
    .rule_type == "pull_request" and
    .result == "fail"
  )] | length == 1;

.result == "bypass" and expected_default_pull_request_rule($ruleset_id)
