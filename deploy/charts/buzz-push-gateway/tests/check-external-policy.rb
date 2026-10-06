#!/usr/bin/env ruby
# Helm post-renderer: validate external runtime policy ownership in the full release.
require 'yaml'
input = STDIN.read
resources = YAML.parse_stream(input).children.map do |doc|
  stream = Psych::Nodes::Stream.new
  stream.children << doc
  YAML.safe_load(stream.to_yaml, permitted_classes: [], aliases: false)
end.compact
annotation = 'buzz.block.xyz/external-network-policy'
resources.select { |r| r['kind'] == 'Deployment' }.each do |deployment|
  name = deployment.dig('metadata', 'annotations', annotation)
  next unless name
  release_namespace = deployment.dig('metadata', 'annotations', 'buzz.block.xyz/release-namespace')
  abort "external policy #{name}: missing release namespace" if !release_namespace.is_a?(String) || release_namespace.empty?
  namespace = deployment.dig('metadata', 'namespace')
  namespace = release_namespace if namespace.nil? || namespace.empty?
  policies = resources.select do |r|
    r['apiVersion'] == 'networking.k8s.io/v1' && r['kind'] == 'NetworkPolicy' &&
      r.dig('metadata', 'name') == name &&
      (r.dig('metadata', 'namespace').to_s.empty? ? release_namespace : r.dig('metadata', 'namespace')) == namespace
  end
  abort "external policy #{name}: expected exactly one replacement in #{namespace}" unless policies.length == 1
  abort "external policy #{name}: replacement must not be a Helm hook" if policies.first.dig('metadata', 'annotations', 'helm.sh/hook')
  spec = policies.first.fetch('spec')
  labels = deployment.dig('spec', 'template', 'metadata', 'labels')
  selector = spec.fetch('podSelector')
  # Require the immutable runtime identity explicitly, not a namespace-wide policy.
  identity = deployment.dig('spec', 'selector', 'matchLabels')
  match = selector.fetch('matchLabels', {})
  abort "external policy #{name}: missing runtime identity selector" unless identity.all? { |k, v| match[k] == v }
  abort "external policy #{name}: selector does not match runtime" unless match.all? { |k, v| labels[k] == v }
  selector.fetch('matchExpressions', []).each do |expression|
    key = expression.fetch('key')
    values = expression.fetch('values', [])
    matches = case expression.fetch('operator')
              when 'In' then labels.key?(key) && values.include?(labels[key])
              when 'NotIn' then !labels.key?(key) || !values.include?(labels[key])
              when 'Exists' then labels.key?(key)
              when 'DoesNotExist' then !labels.key?(key)
              else false
              end
    abort "external policy #{name}: expression does not match runtime" unless matches
  end
  abort "external policy #{name}: must isolate ingress and egress" unless %w[Ingress Egress].all? { |t| spec.fetch('policyTypes', []).include?(t) }
end
# Preserve Helm output byte-for-byte only after every deployment passes.
STDOUT.write(input)
