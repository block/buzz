import { createCliRenderer } from '@opentui/core';
import { OpenTuiScreen } from '../src/opentui-screen.ts';

// Explicit presentation-only fixture. No production config, credential, relay,
// provider or host module is imported, regardless of HOME or inherited key vars.
const renderer = await createCliRenderer({ exitOnCtrlC: false, screenMode: 'alternate-screen', consoleMode: 'disabled', openConsoleOnError: false });
const view = new OpenTuiScreen(renderer);
function show(scope = 0) {
  view.show([{ id: 'fixture', label: scope ? 'Synthetic host observation' : 'Synthetic Local Host', detail: scope ? 'Agents: fixture only. Independent catalog: Not available in this build. No relay request is made.' : 'This is a synthetic presentation fixture. No host started. No native credentials or transport constructed.' }], [
    { label: 'Edit public label', run: async () => { const answer = await view.input('Public label', 'Fixture'); view.notice(answer === undefined ? 'Cancelled; no write.' : `Fixture accepted: ${answer}`); } },
    { label: 'Hidden fixture key', run: async () => { const answer = await view.input('Synthetic key only', '', true); view.notice(answer === undefined ? 'Cancelled; volatile input cleared.' : 'Synthetic input accepted; not persisted.'); } },
    { label: 'Quit manager (not Stop)', run: () => view.close() },
  ]);
}
view.onScope = show;
const stop = () => view.close();
process.on('SIGINT', stop); process.on('SIGTERM', stop);
show();
await view.done;
process.off('SIGINT', stop); process.off('SIGTERM', stop);
