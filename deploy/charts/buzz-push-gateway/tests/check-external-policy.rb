#!/usr/bin/env ruby
# Validate the complete release; strict external callers supply --expect.
require 'yaml'
require 'date'
# Strict callers provide expectations independently of rendered annotations.
expectations = []
until ARGV.empty?
  abort 'usage: check-external-policy.rb [--expect DEPLOYMENT POLICY NAMESPACE]...' unless ARGV.shift == '--expect' && ARGV.length >= 3
  expectations << ARGV.shift(3)
end
input = STDIN.read
resources = YAML.parse_stream(input).children.map do |doc|
  stream = Psych::Nodes::Stream.new
  stream.children << doc
  YAML.safe_load(stream.to_yaml, permitted_classes: [Date, Time], aliases: true)
end.compact
# Match Kubernetes Unstructured.IsList: an items array, including typed lists.
# https://github.com/kubernetes/apimachinery/blob/v0.34.1/pkg/apis/meta/v1/unstructured/unstructured.go
def flatten_resources(resource, enclosing_hooks = {})
  annotations = resource.dig('metadata', 'annotations') || {}
  hooks = enclosing_hooks.merge(annotations.select { |k, _| %w[helm.sh/hook argocd.argoproj.io/hook].include?(k) })
  if resource['items'].is_a?(Array)
    resource.fetch('items').flat_map { |item| flatten_resources(item, hooks) }
  else
    unless hooks.empty?
      resource = Marshal.load(Marshal.dump(resource))
      resource['metadata'] ||= {}
      resource['metadata']['annotations'] = annotations.merge(hooks)
    end
    [resource]
  end
end
resources = resources.flat_map { |resource| flatten_resources(resource) }
def label_value?(value)
  value.is_a?(String) && value.length <= 63 && (value.empty? || /\A[A-Za-z0-9](?:[-_.A-Za-z0-9]*[A-Za-z0-9])?\z/.match?(value))
end

def label_key?(key)
  return false unless key.is_a?(String)
  parts = key.split('/', -1)
  return false unless [1, 2].include?(parts.length) && !parts.last.empty? && label_value?(parts.last)
  return true if parts.length == 1
  prefix = parts.first
  prefix.length <= 253 && /\A[a-z0-9](?:[-a-z0-9]*[a-z0-9])?(?:\.[a-z0-9](?:[-a-z0-9]*[a-z0-9])?)*\z/.match?(prefix)
end
annotation = 'buzz.block.xyz/external-network-policy'
expectations.each do |deployment_name, policy_name, namespace|
  candidates = resources.select do |r|
    r['kind'] == 'Deployment' && r.dig('metadata', 'name') == deployment_name &&
      (r.dig('metadata', 'namespace').to_s.empty? ? namespace : r.dig('metadata', 'namespace')) == namespace
  end
  abort "external policy #{policy_name}: expected exactly one deployment #{deployment_name}" unless candidates.length == 1
  annotations = candidates.first.dig('metadata', 'annotations') || {}
  abort "external policy #{policy_name}: missing or overwritten ownership marker" unless annotations[annotation] == policy_name && annotations['buzz.block.xyz/release-namespace'] == namespace
end
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
  abort "external policy #{name}: replacement must not be an Argo CD hook" if policies.first.dig('metadata', 'annotations', 'argocd.argoproj.io/hook')
  spec = policies.first.fetch('spec')
  labels = deployment.dig('spec', 'template', 'metadata', 'labels')
  selector = spec.fetch('podSelector')
  # Require the immutable runtime identity explicitly, not a namespace-wide policy.
  identity = deployment.dig('spec', 'selector', 'matchLabels')
  match = selector.fetch('matchLabels', {})
  expressions = selector.fetch('matchExpressions', [])
  identity_bound = identity.all? do |k, v|
    match[k] == v || expressions.any? { |e| e['key'] == k && e['operator'] == 'In' && e['values'] == [v] }
  end
  abort "external policy #{name}: missing runtime identity selector" unless identity_bound
  abort "external policy #{name}: selector does not match runtime" unless match.all? { |k, v| labels[k] == v }
  selector.fetch('matchExpressions', []).each do |expression|
    key = expression.fetch('key')
    values = expression.fetch('values', [])
    operator = expression.fetch('operator')
    valid_values = values.is_a?(Array) && values.all? { |v| label_value?(v) }
    valid_values &&= %w[In NotIn].include?(operator) ? !values.empty? : (%w[Exists DoesNotExist].include?(operator) && values.empty?)
    abort "external policy #{name}: invalid selector expression" unless label_key?(key) && valid_values
    matches = case operator
              when 'In' then labels.key?(key) && values.include?(labels[key])
              when 'NotIn' then !labels.key?(key) || !values.include?(labels[key])
              when 'Exists' then labels.key?(key)
              when 'DoesNotExist' then !labels.key?(key)
              else false
              end
    abort "external policy #{name}: expression does not match runtime" unless matches
  end
  types = spec['policyTypes']
  if types.nil? || types.empty?
    types = ['Ingress']
    types << 'Egress' unless spec.fetch('egress', []).empty?
  end
  abort "external policy #{name}: must isolate ingress and egress" unless %w[Ingress Egress].all? { |t| types.include?(t) }
end
# Preserve Helm output byte-for-byte only after every deployment passes.
STDOUT.write(input)
