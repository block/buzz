import manifest from './model-capabilities.json' with { type: 'json' };

/** Packaged verbatim from scripts/model-capabilities.json. Interpretation mirrors
 * Desktop modelCapabilities.ts exact/family precedence. No fallback effort picker:
 * unknown custom names and UC FQNs deliberately omit tuning rather than guessing. */
export function runtimeCapabilities(provider: string, model: string) {
  const canon = provider === 'openai-compat' ? 'openai' : provider;
  if (canon === 'databricks_v2' && model.split('.').length === 3 && model.split('.').every(v => v.length > 0 && !/[\s/]/.test(v))) {
    const service = model.split('.')[2]!.toLowerCase();
    const tokens = manifest.family_tokens.flatMap(token => {
      let from = 0;
      while (true) {
        const i = service.indexOf(token, from); if (i < 0) return [];
        if (i === 0 || !/[a-z0-9]/.test(service[i-1]!)) return [i];
        from = i + 1;
      }
    });
    const stripped = service.slice(tokens.length ? Math.min(...tokens) : 0);
    const version = /^gpt-(\d+)(?=[^a-z0-9]|$)/.exec(stripped);
    return { ...manifest.provider_fallbacks.databricks_v2.concrete_unknown, supported_efforts: [] as string[], databricks_v2_wire_route: version && Number(version[1]) >= 5 ? 'openai-responses' : 'mlflow-chat' };
  }
  const lower = model.toLowerCase();
  const exact = manifest.exact_records.find(r => r.provider === canon && r.raw_model_id.toLowerCase() === lower);
  if (exact) return exact;
  let start = lower.length;
  for (const token of manifest.family_tokens) {
    let from = 0;
    while (true) {
      const index = lower.indexOf(token, from); if (index < 0) break;
      if (index === 0 || !/[a-z0-9]/.test(lower[index - 1]!)) { start = Math.min(start, index); break; }
      from = index + 1;
    }
  }
  const stripped = start === lower.length ? lower : lower.slice(start);
  const matches = manifest.family_rules.flatMap(rule => {
    if (!rule.providers.includes(canon)) return [];
    const tokens = [rule.match_value, ...('match_aliases' in rule ? rule.match_aliases as string[] : [])];
    const lengths = tokens.filter(token => rule.match_kind === 'exact' ? stripped === token : stripped.startsWith(token) && (!stripped[token.length] || !/[a-z0-9]/.test(stripped[token.length]!))).map(token => token.length);
    return lengths.length ? [{ rule, length: Math.max(...lengths) }] : [];
  }).sort((a, b) => b.length - a.length || (a.rule.id < b.rule.id ? -1 : 1));
  const rule = matches[0]?.rule;
  if (rule) return rule;
  const fallback = manifest.provider_fallbacks[canon as keyof typeof manifest.provider_fallbacks]?.concrete_unknown;
  return fallback ? { ...fallback, supported_efforts: [] as string[] } : undefined;
}

export function runtimeEfforts(provider: string, model: string): readonly string[] {
  const result = runtimeCapabilities(provider, model);
  return !result || result.thinking_mode === 'omit-fields' ? [] : result.supported_efforts;
}

/** Recheck at save AND actual launch; no caller can inject a universal level. */
export function validateRuntimeEffort(provider: string, model: string, effort?: string) {
  if (effort !== undefined && !runtimeEfforts(provider, model).includes(effort)) throw Error('Effort is not supported by this provider/model');
}
