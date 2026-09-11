import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createTestRenderer } from '@opentui/core/testing';
import { OpenTuiScreen } from '../src/opentui-screen.ts';

test('actual OpenTUI selection, hidden entry, multiline save, resize and quit', async () => {
  const ui = await createTestRenderer({ width: 60, height: 24, exitOnCtrlC: false });
  const view = new OpenTuiScreen(ui.renderer);
  try {
    let selected = '';
    view.onSelect = id => { selected = id; };
    view.show([{ id: 'a', label: 'Synthetic A', detail: 'First detail' }, { id: 'b', label: 'Synthetic B', detail: 'Second detail' }], []);
    await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /Local Host.*Agents/);
    ui.mockInput.pressArrow('down');
    await ui.renderOnce();
    assert.equal(selected, 'b');
    assert.match(ui.captureCharFrame(), /Second detail/);
    const secret = 'synthetic-secret-not-a-real-key';
    const entered = view.input('Fixture secret', '', true);
    await ui.mockInput.typeText(secret);
    await ui.renderOnce();
    assert.ok(!ui.captureCharFrame().includes(secret));
    ui.mockInput.pressEnter();
    assert.equal(await entered, secret);
    const cancelled = view.input('Cancel this secret', '', true);
    await ui.mockInput.pasteBracketedText(secret);
    ui.mockInput.pressEscape();
    assert.equal(await cancelled, undefined);
    const edit = view.input('Private draft', 'line one', false, true);
    ui.mockInput.pressKey('s', { ctrl: true });
    assert.equal(await edit, 'line one');
    ui.resize(30, 10); await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /Terminal too small/);
    ui.resize(60, 24); await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /Local Host.*Agents/);
    const quitting = view.input('Quit clears volatile input', '', true);
    await ui.mockInput.typeText(secret);
    ui.mockInput.pressKey('q', { ctrl: true });
    await view.done;
    assert.equal(await quitting, undefined);
  } finally { view.close(); }
});
