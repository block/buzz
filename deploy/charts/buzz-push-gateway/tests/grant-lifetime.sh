#!/usr/bin/env bash
set -euo pipefail
fixture=$(mktemp)
trap 'rm -f "$fixture"' EXIT

# Use real Helm renders, including YAML values (not just --set, which can
# preserve a different numeric type). No deployment or real secrets are needed.
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -rjson -ryaml -ropen3 \
  - "$fixture" <<'RUBY'
chart = 'deploy/charts/buzz-push-gateway'
command = ['helm', 'template', 'push', chart]
cases = []
render = lambda do |label, expected, args, values|
  output, error, status = Open3.capture3(*command, *args, stdin_data: values)
  raise "#{label}: Helm failed: #{error}" unless status.success?
  deployment = YAML.load_stream(output).compact.find { |x| x['kind'] == 'Deployment' }
  entries = deployment.fetch('spec').fetch('template').fetch('spec').fetch('containers')[0].fetch('env')
  # Secret references are intentionally excluded; Config's fixture supplies
  # synthetic keyrings and database configuration in memory.
  literal_env = entries.select { |entry| entry.key?('value') }.to_h do |entry|
    value = entry.fetch('value')
    raise "#{label}: #{entry['name']} is not a string" unless value.is_a?(String)
    [entry.fetch('name'), value]
  end
  raise "#{label}: lifetime missing" unless literal_env.key?('BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS')
  cases << [label, expected, literal_env]
end
render.call('default', 2592000, [], '')
[1, 2592000, 2592001, 31536000].each do |seconds|
  render.call("values #{seconds}", seconds, ['-f', '-'], "maxGrantLifetimeSeconds: #{seconds}\n")
  render.call("set #{seconds}", seconds, ['--set', "maxGrantLifetimeSeconds=#{seconds}"], '')
end

# Integer conversion must never hide invalid inputs. Helm's schema remains
# authoritative, including rejecting strings that merely look numeric.
['0', '-1', '31536001', '2592000.5', '"2592000"', 'true'].each do |value|
  _, error, status = Open3.capture3(*command, '-f', '-', stdin_data: "maxGrantLifetimeSeconds: #{value}\n")
  unless !status.success? && error.include?('maxGrantLifetimeSeconds')
    raise "expected schema rejection for #{value}: #{error}"
  end
end
File.write(ARGV.fetch(0), JSON.generate(cases))
RUBY

BUZZ_TEST_HELM_ENV_FILE="$fixture" cargo test -p buzz-push-gateway --lib \
  config::tests::helm_rendered_grant_lifetimes -- --ignored --exact
