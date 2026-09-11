import { createTestRenderer } from '@opentui/core/testing';
import { OpenTuiScreen } from '../src/opentui-screen.ts';
import { writeFileSync } from 'node:fs';
const output = process.argv[2];
if (!output) throw Error('Pass existing output directory with trailing slash');
const ui = await createTestRenderer({ width: 100, height: 30, exitOnCtrlC: false });
const view = new OpenTuiScreen(ui.renderer);
const save = async (name: string) => { await ui.renderOnce(); writeFileSync(output + name + '.txt', ui.captureCharFrame()); writeFileSync(output + name + '-spans.json', JSON.stringify(ui.captureSpans())); };
try {
 ui.mockInput.pressArrow('right');
 view.show([{ id: 'synthetic-agent', label: 'Agent 12345678…abcdef', detail: 'Agent 12345678…abcdef\nHost abcdef12…123456\nUNKNOWN — stale or unreachable; last reported below\nReported phase: running\n\nActual run (reported)\nConfiguration: daily\nModel: Not reported\n\nSelected for next Start\nConfiguration: careful\nChoosing next does not change the actual run.', evidence: '{"syntheticPublicEvidence":true}' }], [{ label: 'Start agent', disabled: 'Host stale or unreachable. Refresh before acting.', run: () => { throw Error('Must never run'); } }, { label: 'Restart', disabled: 'Backend Restart may switch bindings. Use Stop, inspect accepted stopped report, then Start. No Restart request is sent.', run: () => { throw Error('Must never run'); } }]);
 await save('fixture-agent');
 view.setPending(true); view.notice('Working: synthetic request… Esc cancels waiting, not committed OS writes or remote work.');
 ui.mockInput.pressKey('F2'); await save('fixture-pending');
 ui.mockInput.pressKey('q', { ctrl: true });
} finally { view.close(); }
