import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createTestRenderer } from '@opentui/core/testing';
import { OpenTuiScreen } from '../src/opentui-screen.ts';

test('host settings has right-pane controls, no inspection menu, and hidden ordinary-key entry', async () => {
  const ui = await createTestRenderer({ width: 100, height: 30, exitOnCtrlC: false });
  const view = new OpenTuiScreen(ui.renderer);
  try {
    let calls = 0;
    view.show(['Agents','Providers','Runtimes'].map(label => ({ id: label,label,detail: 'PRIVATE DETAIL MUST NOT DISPLAY' })),['Register agent','Add provider','Add runtime','Start','Stop'].map(label => ({ label,run: () => { calls++; } })));
    view.setRelay('wss://fixture.invalid','disconnected'); await ui.renderOnce();
    const frame = ui.captureCharFrame();
    assert.match(frame,/Host settings/); assert.match(frame,/Register agent/); assert.match(frame,/wss:\/\/fixture.invalid.*disconnected/);
    assert.ok(!frame.includes('Inspect')); assert.ok(!frame.includes('PRIVATE DETAIL')); assert.ok(!frame.includes('Actions (a)'));
    ui.mockInput.pressTab(); ui.mockInput.pressEnter(); await ui.renderOnce(); assert.equal(calls,1);
    const secret = 'synthetic-aq?io-private-value';
    const entered = view.input('Hidden fixture key','',true);
    await ui.mockInput.typeText(secret); await ui.renderOnce(); assert.ok(!ui.captureCharFrame().includes(secret)); assert.equal(calls,1);
    ui.mockInput.pressEnter(); assert.equal(await entered,secret);
    const cancelled = view.input('Cancel key','',true); await ui.mockInput.pasteBracketedText(secret); ui.mockInput.pressEscape(); assert.equal(await cancelled,undefined);
    const ordinary = view.input('Public text'); await ui.mockInput.typeText('aq?io'); ui.mockInput.pressEnter(); assert.equal(await ordinary,'aq?io');
    ui.resize(44,24); await ui.renderOnce(); assert.match(ui.captureCharFrame(),/Enter Open/); assert.match(ui.captureCharFrame(),/Register/);
    ui.resize(30,10); await ui.renderOnce(); assert.match(ui.captureCharFrame(),/Resize to at least 40 × 16/);
  } finally { view.close(); }
});

test('disabled host control and pending cancellation never run an operation; multiline/quit retain editor ownership', async () => {
  const ui = await createTestRenderer({ width: 100,height: 30,exitOnCtrlC: false }); const view = new OpenTuiScreen(ui.renderer);
  try {
    let calls = 0, cancelled = 0; view.onCancel = () => { cancelled++; };
    view.show([{id:'agents',label:'Agents',detail:''}],[{label:'Stop',disabled:'Ownership unknown.',run:()=>{calls++;}}]);
    ui.mockInput.pressTab(); ui.mockInput.pressEnter(); await ui.renderOnce(); assert.equal(calls,0); assert.match(ui.captureCharFrame(),/Ownership unknown/);
    view.setPending(true); ui.mockInput.pressEscape(); await new Promise(resolve => setTimeout(resolve,50)); await ui.renderOnce(); assert.equal(cancelled,1); view.setPending(false);
    const edit = view.input('Draft','line one',false,true); ui.mockInput.pressKey('s',{ctrl:true}); assert.equal(await edit,'line one');
    const quitting = view.input('Hidden','',true); await ui.mockInput.typeText('q');
    assert.equal(await Promise.race([view.done.then(()=>'quit'),new Promise(resolve=>setTimeout(resolve,40,'open'))]),'open');
    ui.mockInput.pressKey('q',{ctrl:true}); await view.done; assert.equal(await quitting,undefined);
  } finally { view.close(); }
});
