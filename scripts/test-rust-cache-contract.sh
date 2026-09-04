#!/usr/bin/env bash
set -euo pipefail

root=${BUZZ_RUST_CACHE_CONTRACT_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
renovate="$root/renovate.json"
known_good='e18b497796c12c097a38f9edb9d0641fb99eee32'
known_bad='6323deb102c322ba6fcbdcafc7e3dddab59af2b6'

ruby - "$root" "$renovate" "$known_good" "$known_bad" <<'RUBY'
require "json"
require "pathname"
require "yaml"

root_name, renovate_name, known_good, known_bad = ARGV
root = Pathname(root_name)
workflow_paths = (root / ".github" / "workflows").children
  .select { |path| path.file? && [".yml", ".yaml"].include?(path.extname) }
  .sort
abort "no GitHub Actions workflows found" if workflow_paths.empty?

load_workflow = lambda do |path|
  workflow = YAML.safe_load(path.binread.force_encoding("UTF-8"), aliases: true)
  abort "#{path}: workflow must be a YAML mapping" unless workflow.is_a?(Hash)
  workflow
rescue Psych::Exception => error
  abort "#{path}: invalid workflow YAML: #{error.message}"
end

uses_values = []
walk = nil
walk = lambda do |value, path, location|
  case value
  when Hash
    value.each do |key, child|
      child_location = "#{location}.#{key}"
      uses_values << [path, child_location, child] if key.to_s == "uses"
      walk.call(child, path, child_location)
    end
  when Array
    value.each_with_index do |child, index|
      walk.call(child, path, "#{location}[#{index}]")
    end
  end
end

workflows = workflow_paths.to_h do |path|
  workflow = load_workflow.call(path)
  walk.call(workflow, path, "$")
  [path, workflow]
end

loaded_paths = workflow_paths.to_h { |path| [path.cleanpath, true] }
uses_index = 0
while uses_index < uses_values.length
  coordinate = uses_values[uses_index][2]
  uses_index += 1
  next unless coordinate.is_a?(String) && coordinate.start_with?("./")

  clean_root = root.realpath
  target = (clean_root / coordinate.delete_prefix("./")).cleanpath
  resolved_target = target.exist? ? target.realpath : target
  root_prefix = "#{clean_root}#{File::SEPARATOR}"
  inside_root = resolved_target == clean_root || resolved_target.to_s.start_with?(root_prefix)
  abort "local action path escapes contract root: #{coordinate}" unless inside_root
  next unless target.directory?

  manifest = [target / "action.yml", target / "action.yaml"].find(&:file?)
  abort "local action manifest missing: #{coordinate}" unless manifest
  clean_manifest = manifest.cleanpath
  next if loaded_paths[clean_manifest]

  loaded_paths[clean_manifest] = true
  action = load_workflow.call(clean_manifest)
  walk.call(action, clean_manifest, "$")
end

cache_uses = uses_values.each_with_object([]) do |(path, location, coordinate), found|
  next unless coordinate.is_a?(String)

  match = /\ASwatinem\/rust-cache@(.+)\z/i.match(coordinate)
  found << [path, location, match[1]] if match
end
abort "no Swatinem/rust-cache actions found" if cache_uses.empty?

cache_uses.each do |path, location, ref|
  if ref == known_bad
    abort "#{path}:#{location}: restored rust-cache v2.9.2, which poisons sherpa caches"
  end
  unless ref == known_good
    abort "#{path}:#{location}: rust-cache must stay on the v2.9.1 digest"
  end
end

rust_ci_path = root / ".github" / "workflows" / "_ci-rust.yml"
rust_ci = workflows.fetch(rust_ci_path) { load_workflow.call(rust_ci_path) }
jobs = rust_ci["jobs"]
unit_tests = jobs.is_a?(Hash) ? jobs["unit-tests"] : nil
abort "unit-tests job missing" unless unit_tests.is_a?(Hash)
steps = unit_tests["steps"]
abort "Unit Tests steps missing" unless steps.is_a?(Array)
cache_steps = steps.select do |step|
  step.is_a?(Hash) && step["uses"].is_a?(String) &&
    step["uses"].match?(/\ASwatinem\/rust-cache@/i)
end
abort "Unit Tests must contain exactly one rust-cache action step" unless cache_steps.length == 1

cache_with = cache_steps.first["with"]
unless cache_with.is_a?(Hash) && cache_with["key"] == "sherpa-cache-v1"
  abort "Unit Tests rust-cache action must keep with.key set to sherpa-cache-v1"
end

renovate = JSON.parse(Pathname(renovate_name).binread.force_encoding("UTF-8"))
rules = renovate.fetch("packageRules", []).select do |rule|
  rule["matchManagers"] == ["github-actions"] &&
    rule["matchPackageNames"] == ["Swatinem/rust-cache"]
end
unless rules.length == 1 && rules.first["allowedVersions"] == "<=2.9.1"
  abort "Renovate must keep Swatinem/rust-cache at v2.9.1 or older"
end
RUBY

echo "rust cache contract passed"
