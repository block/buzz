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
    assert.match(ui.captureCharFrame(), /\? Help · a Actions · i Inspect · o Status · q Quit/, 'ordinary-key footer, no function keys');
    ui.resize(44, 24); await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /\? a Actions i Inspect o Status q Quit/, 'compact ordinary-key footer fits the 40-column floor');
    ui.resize(60, 24); await ui.renderOnce();
    ui.mockInput.pressArrow('down');
    await ui.renderOnce();
    assert.equal(selected, 'b');
    assert.ok(!ui.captureCharFrame().includes('Second detail'));
    ui.mockInput.pressEnter(); await ui.renderOnce();
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
    assert.match(ui.captureCharFrame(), /Resize to at least 40 × 16/);
    ui.resize(60, 24); await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /Local Host.*Agents/);
    const quitting = view.input('Quit clears volatile input', '', true);
    await ui.mockInput.typeText('aq?io');
    ui.mockInput.pressKey('q'); // plain q inside a secret form types text; it must never quit or fire actions
    await ui.renderOnce();
    assert.equal(await Promise.race([view.done.then(() => 'quit'), new Promise(resolve => setTimeout(resolve, 120, 'open'))]), 'open', 'plain q must not quit an open form');
    assert.ok(!ui.captureCharFrame().includes('[Actions]'), 'a must not open the drawer from a secret form');
    ui.mockInput.pressKey('q', { ctrl: true });
    await view.done;
    assert.equal(await quitting, undefined);
  } finally { view.close(); }
});

test('drawer guards, persistent keys, pending Esc priority, list navigation and validated input', async () => {
  const ui = await createTestRenderer({ width: 100, height: 30, exitOnCtrlC: false });
  const view = new OpenTuiScreen(ui.renderer);
  try {
    let calls = 0, cancelled = 0, selected = '';
    view.onSelect = id => { selected = id; };
    view.onCancel = () => { cancelled++; };
    view.show(Array.from({length: 25}, (_, i) => ({id: String(i), label: `Agent ${i}`, detail: `Summary ${i}`, evidence: `PUBLIC ${i}`})), [
      {label: 'Start', disabled: 'Requires fresh stopped report.', run: () => { calls++; }},
      ...Array.from({length: 10}, (_, i) => ({label: `Action ${i}`, run: () => { calls++; }})),
    ]);
    await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /Actions \(a\)/, 'visible named control shows the ordinary key');
    assert.match(ui.captureCharFrame(), /Inspect \(i\)/);
    ui.mockInput.pressKey('HOME'); await ui.renderOnce();
    await ui.mockMouse.scroll(4, 5, 'down'); await ui.renderOnce();
    assert.equal(selected, '1');
    ui.mockInput.pressKey('END'); await ui.renderOnce();
    assert.equal(selected, '24');
    await ui.mockMouse.click(4, 5); await ui.renderOnce();
    assert.equal(selected, '24', 'scrolled click cannot select wrong row');
    ui.mockInput.pressEnter(); assert.equal(calls, 0);
    ui.mockInput.pressKey('a'); await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /\(unavailable\) Start/);
    assert.match(ui.captureCharFrame(), /Requires fresh stopped report/);
    ui.mockInput.pressEnter(); assert.equal(calls, 0);
    ui.mockInput.pressKey('END'); await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /Action 9/);
    ui.mockInput.pressEscape(); await new Promise(resolve => setTimeout(resolve, 50));
    view.setPending(true); view.notice('Working: synthetic pending request');
    ui.mockInput.pressKey('a'); await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /Esc stops waiting/);
    assert.match(ui.captureCharFrame(), /\? Help · a Actions · i Inspect · o Status · q Quit/);
    ui.mockInput.pressEnter(); assert.equal(calls, 0);
    ui.mockInput.pressEscape(); await new Promise(resolve => setTimeout(resolve, 50)); assert.equal(cancelled, 1);
    await ui.renderOnce(); assert.match(ui.captureCharFrame(), /\[Actions\]/);
    view.setPending(false); ui.mockInput.pressEscape(); await new Promise(resolve => setTimeout(resolve, 50));
    ui.mockInput.pressKey('i'); await ui.renderOnce(); assert.match(ui.captureCharFrame(), /PUBLIC 24/);
    ui.mockInput.pressEscape(); await new Promise(resolve => setTimeout(resolve, 50));
    const input = view.input('Required public field', '', false, false, value => value ? '' : 'Required; enter a value.');
    ui.mockInput.pressEnter(); await ui.renderOnce(); assert.match(ui.captureCharFrame(), /Required; enter a value/);
    await ui.mockInput.typeText('retained public input');
    ui.resize(60,24); await ui.renderOnce(); assert.match(ui.captureCharFrame(), /retained public input/);
    ui.mockInput.pressEnter(); assert.equal(await input, 'retained public input');
    view.notice('FAILED: a long outcome\n' + 'Details\n'.repeat(40));
    ui.mockInput.pressKey('o'); await ui.renderOnce(); assert.match(ui.captureCharFrame(), /FAILED: a long outcome/);
    assert.match(ui.captureCharFrame(), /\? Help · a Actions · i Inspect · o Status · q Quit/);
    ui.mockInput.pressEscape(); await new Promise(resolve => setTimeout(resolve, 50));
    ui.mockInput.pressKey('?'); await ui.renderOnce();
    assert.match(ui.captureCharFrame(), /Help · Esc close/);
    assert.match(ui.captureCharFrame(), /Shortcuts do not run while you edit a field/);
    ui.mockInput.pressEscape(); await new Promise(resolve => setTimeout(resolve, 50));
    // Legacy F-key compatibility aliases remain functional, undocumented in footer/help.
    ui.mockInput.pressKey('F2'); await ui.renderOnce(); assert.match(ui.captureCharFrame(), /\[Actions\]/);
    ui.mockInput.pressEscape(); await new Promise(resolve => setTimeout(resolve, 50));
    ui.mockInput.pressKey('F3'); await ui.renderOnce(); assert.match(ui.captureCharFrame(), /PUBLIC 24/);
    ui.mockInput.pressEscape(); await new Promise(resolve => setTimeout(resolve, 50));
    ui.mockInput.pressKey('q');
    assert.equal(await Promise.race([view.done.then(() => 'quit'), new Promise(resolve => setTimeout(resolve, 2000, 'open'))]), 'quit', 'plain q quits outside editable contexts');
  } finally { view.close(); }
});

test('ordinary shortcut characters type as text in public, secret and multiline inputs', async () => {
  const ui = await createTestRenderer({ width: 100, height: 30, exitOnCtrlC: false });
  const view = new OpenTuiScreen(ui.renderer);
  try {
    view.show([{ id: 'row', label: 'Row', detail: 'Detail', evidence: 'EVIDENCE' }], []);
    await ui.renderOnce();
    const publicField = view.input('Public field', '', false, false);
    await ui.mockInput.typeText('a?ioq');
    await ui.renderOnce();
    assert.ok(!ui.captureCharFrame().includes('[Actions]'), 'a must not open the drawer from a public form');
    assert.ok(!ui.captureCharFrame().includes('Technical details'), 'i must not open inspect from a public form');
    assert.ok(!ui.captureCharFrame().includes('No status message yet'), 'o must not open the outcome overlay from a public form');
    ui.mockInput.pressEnter();
    assert.equal(await publicField, 'a?ioq');
    const secret = view.input('Hidden field', '', true);
    await ui.renderOnce();
    assert.ok(!ui.captureCharFrame().includes('a?ioq'), 'prior public field value not left on screen');
    await ui.mockInput.typeText('a?ioq');
    ui.mockInput.pressEnter();
    assert.equal(await secret, 'a?ioq');
    await ui.renderOnce();
    assert.ok(!ui.captureCharFrame().includes('a?ioq'), 'hidden value never rendered');
    const draft = view.input('Multiline draft', '', false, true);
    await ui.mockInput.typeText('a io q?');
    ui.mockInput.pressKey('s', { ctrl: true });
    assert.equal(await draft, 'a io q?');
  } finally { view.close(); }
});
