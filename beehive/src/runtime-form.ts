import { runtimeEfforts } from './runtime-effort.ts';
import type { ManagerSnapshot } from './manager-controller.ts';

/** Same short form used by the actual renderer and synthetic key/form fixtures. */
export async function runtimeForm(screen: {
  choose(label: string, options: string[]): Promise<string | undefined>;
  input(label: string, initial?: string): Promise<string | undefined>;
  confirm(label: string): Promise<boolean>;
  notice(text: string): void;
}, snapshot: () => ManagerSnapshot, request: (action: string, values?: Record<string,string>) => Promise<void>) {
  await request('runtime-form');
  const harnesses = snapshot().harnesses ?? [];
  if (!harnesses.length) { screen.notice(snapshot().status); return; }
  const labels = harnesses.map(h => `${h.label} · ${h.providers.length ? 'Available' : h.state === 'available' ? 'Provider integration unavailable' : h.state}`);
  const harness = await screen.choose('Harness · detection is not sign-in', labels); if (!harness) return;
  const selected = harnesses[labels.indexOf(harness)];
  if (!selected?.providers.length) { screen.notice(selected?.reason ?? 'Unsupported harness'); return; }
  const providers = (snapshot().settings?.providers ?? []).filter(p => selected.providers.includes(p.type));
  if (!providers.length) { screen.notice('Add a supported provider first.'); return; }
  const choice = await screen.choose('Provider', providers.map(p => `${p.name} · ${p.id}`)); if (!choice) return;
  const provider = providers.find(p => choice === `${p.name} · ${p.id}`)!;
  await request('models', { provider: provider.id });
  const models = snapshot().models;
  const choiceModel = await screen.choose(models ? 'Model' : 'Model listing unavailable · Custom model allowed', [...(models ?? []), 'Custom model']); if (!choiceModel) return;
  const model = choiceModel === 'Custom model' ? await screen.input('Exact model ID. Custom does not verify provider access.') : choiceModel; if (!model) return;
  const efforts = selected.id === 'buzz-agent' ? runtimeEfforts(provider.type, model) : [];
  const effort = efforts.length ? await screen.choose('Effort · new runs only', ['Inherit', ...efforts]) : 'Inherit'; if (!effort) return;
  const name = await screen.input('Runtime name', model); if (!name) return;
  if (await screen.confirm(`Save ${name}?\n${selected.label} · ${provider.name} · ${model}\n${efforts.length ? `Effort: ${effort}` : 'No supported effort control for this model.'}\nSaved for new runs. Running agents do not change.`)) await request('add-runtime', { name, model, provider: provider.id, harness: selected.id, ...(effort !== 'Inherit' ? { effort } : {}) });
}
