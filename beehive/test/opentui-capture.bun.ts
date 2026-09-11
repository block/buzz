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
 view.show([{ id: 'synthetic-agent', label: 'Agent 12345678…abcdef', detail: 'Agent 12345678…abcdef\nHost abcdef12…123456\nCurrent status unknown. The report is old or the host cannot be reached. Last report:\nReported status: running\n\nCurrent run (host report)\nConfiguration: daily\nModel: Not reported\n\nConfiguration for next start\nConfiguration: careful\nThis choice does not change the current run.', evidence: '{"syntheticPublicEvidence":true}' }], [{ label: 'Start agent', disabled: 'The host report is old or the host cannot be reached. Wait for a recent report before you act.', run: () => { throw Error('Must never run'); } }, { label: 'Restart', disabled: 'Restart can change the local setup. Use Stop. Inspect a recent host report that confirms the agent is stopped. Then use Start. No Restart request is sent.', run: () => { throw Error('Must never run'); } }]);
 await save('fixture-agent');
 view.setPending(true); view.notice('Working… Esc stops waiting. Remote work and saved changes are not cancelled. Inspect before you try again.');
 ui.mockInput.pressKey('a'); await save('fixture-pending');
 ui.mockInput.pressKey('q', { ctrl: true });
} finally { view.close(); }
