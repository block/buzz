import { realpathSync } from 'node:fs';
import type { Interface } from 'node:readline/promises';
import { readCustom, diagnosticReason, type CustomAcp } from './custom-acp.ts';
import { discoverPresets, showPresets } from './presets.ts';
import { text } from './protocol.ts';
import type { Setup } from './host.ts';
/** Local-only definition entry; never print env/argv from owner input. */
export async function customInput(ui: Interface): Promise<CustomAcp> {
  console.log('Custom ACP: owner-only local JSON with id, label, executable, args array, env object, installHint, installInstructionsUrl, contract (diagnostic or goose-native). No install scripts or remote env editing. Goose-native explicitly requires Goose initialize identity, exact native configOptions model and successful native system-prompt extension; arbitrary protocol2 is NOT compatible. Env remains local/private; reserved host/model/auth context cannot be overridden.');
  return readCustom(text(await ui.question('Absolute owner-only custom definition JSON file: ')));
}
/** Build a new immutable local definition without selecting or starting it. */
export async function customBindingInput(ui: Interface, preset: boolean): Promise<Omit<Setup, 'host' | 'ownerSecret' | 'agentSecret'>> {
  let custom: CustomAcp;
  if (preset) {
    showPresets();
    const id = await ui.question('Preset ID: ');
    const selected = discoverPresets().find(p => p.id === id);
    if (!selected) throw Error('Unknown preset ID');
    const executable = selected.executable ?? text(await ui.question(`Missing ${selected.command}; absolute intended local executable (setup only): `));
    custom = { id: selected.id, label: selected.label, executable, args: [...selected.args], env: {}, installHint: selected.installHint, installInstructionsUrl: selected.installInstructionsUrl, contract: 'diagnostic' };
  } else custom = await customInput(ui);
  const workspace = realpathSync(text(await ui.question('Allowed workspace (absolute directory): ')));
  if (custom.contract === 'diagnostic') {
    console.log(diagnosticReason);
    return { mode: 'diagnostic-acp', runner: custom.executable, args: custom.args, custom, workspace, allowedWorkspaces: [workspace] };
  }
  const serviceHome = realpathSync(text(await ui.question('Existing dedicated service HOME (not Desktop HOME): ')));
  const gooseProvider = text(await ui.question('Locally configured Goose provider ID: '));
  const gooseModels = text(await ui.question('Operator-approved compatible exact model IDs (comma-separated): ')).split(',').map(m => m.trim());
  console.log('Local authentication unverified; configure the Goose-compatible adapter as this service user. Exact model/native profile proof required before conversation; no vendor compatibility inferred.');
  return { mode: 'goose', custom, runner: custom.executable, args: custom.args, workspace, allowedWorkspaces: [workspace], serviceHome, configDirectory: serviceHome, gooseProvider, gooseModels };
}
