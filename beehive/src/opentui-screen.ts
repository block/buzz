import { BoxRenderable, InputRenderable, ScrollBoxRenderable, SelectRenderable, TextRenderable, TextareaRenderable, type CliRenderer, type KeyEvent } from '@opentui/core';
import { stripVTControlCharacters } from 'node:util';

export type ManagerRow = { id: string; label: string; detail: string };
export type ManagerAction = { label: string; run: () => void | Promise<void> };
const clean = (value: string) => stripVTControlCharacters(value).replace(/[\x00-\x08\x0b-\x1f\x7f]/g, '');

/** Presentation only: never imports credentials, configuration, transport or host code.
 * Bun owns this renderer; the manager/service interpreter can remain Node. */
export class OpenTuiScreen {
  private readonly root: BoxRenderable;
  private readonly header: TextRenderable;
  private readonly list: SelectRenderable;
  private readonly detail: TextRenderable;
  private readonly scroll: ScrollBoxRenderable;
  private readonly actions: SelectRenderable;
  private readonly status: TextRenderable;
  private readonly small: TextRenderable;
  private rows: ManagerRow[] = [];
  private commands: ManagerAction[] = [];
  private focusIndex = 0;
  private scope = 0;
  private owner = 'signed out';
  private busy = false;
  private closed = false;
  private modal?: { cancel: () => void; key: (key: KeyEvent) => void; paste: (text: string) => void };
  private finish!: () => void;
  readonly done = new Promise<void>(resolve => { this.finish = resolve; });
  onScope: (index: number) => void = () => {};
  onSelect: (id: string) => void = () => {};

  constructor(readonly renderer: CliRenderer) {
    this.root = new BoxRenderable(renderer, { width: '100%', height: '100%', flexDirection: 'column', backgroundColor: '#101820' });
    renderer.root.add(this.root);
    this.header = new TextRenderable(renderer, { height: 2, fg: '#72d9ef', onMouseDown: event => this.switchScope(event.x < renderer.width / 2 ? 0 : 1) });
    this.root.add(this.header);
    const body = new BoxRenderable(renderer, { flexGrow: 1, flexDirection: 'row', minHeight: 1 });
    this.root.add(body);
    this.list = new SelectRenderable(renderer, { width: '32%', height: '100%', showDescription: false, showScrollIndicator: true, selectedBackgroundColor: '#245272', focusedBackgroundColor: '#172d3b', onMouseDown: event => {
      this.focusIndex = 0; this.list.focus();
      const delta = event.y - this.list.y;
      const current = this.list.getSelectedIndex();
      // Select's scrolling offset is private; use relative distance from its visible selection only for unscrolled lists.
      if (this.rows.length <= this.list.height && delta >= 0) this.list.setSelectedIndex(Math.min(this.rows.length - 1, delta));
      else this.list.selectCurrent();
    } });
    body.add(this.list);
    this.scroll = new ScrollBoxRenderable(renderer, { flexGrow: 1, height: '100%', border: true, title: 'Details', scrollY: true, onMouseDown: () => { this.focusIndex = 1; this.scroll.focus(); } });
    body.add(this.scroll);
    this.detail = new TextRenderable(renderer, { width: '100%', content: 'No items.', selectable: false });
    this.scroll.add(this.detail);
    this.actions = new SelectRenderable(renderer, { height: 5, showDescription: false, showScrollIndicator: true, selectedBackgroundColor: '#245272', onMouseDown: event => {
      this.focusIndex = 2; this.actions.focus();
      const delta = event.y - this.actions.y;
      if (this.commands.length <= this.actions.height && delta >= 0) this.actions.setSelectedIndex(Math.min(this.commands.length - 1, delta));
      this.actions.selectCurrent();
    } });
    this.root.add(this.actions);
    this.status = new TextRenderable(renderer, { height: 2, fg: '#ffce75', content: 'Tab pane · arrows select · Enter action · F1 help · Ctrl-Q quit (not Stop)' });
    this.root.add(this.status);
    this.small = new TextRenderable(renderer, { position: 'absolute', top: 0, left: 0, width: '100%', height: '100%', bg: '#101820', content: 'Terminal too small. Resize to 40 × 16. Nothing submitted. Ctrl-Q quits.', visible: false });
    renderer.root.add(this.small);
    this.list.on('selectionChanged', (index: number) => {
      const row = this.rows[index]; this.detail.content = clean(row?.detail ?? 'No items.'); this.scroll.scrollTo(0);
      if (row) this.onSelect(row.id);
    });
    this.actions.on('itemSelected', (index: number) => { void this.run(index); });
    renderer.keyInput.on('keypress', key => this.key(key));
    renderer.keyInput.on('paste', event => {
      if (!this.modal) return;
      this.modal.paste(new TextDecoder().decode(event.bytes));
      // Ordinary editor paste is handled by its focused renderable.
    });
    renderer.on('resize', () => this.resize());
    renderer.on('destroy', () => this.close());
    this.list.focus(); this.heading(); this.resize();
  }

  private resize() { this.small.visible = this.renderer.width < 40 || this.renderer.height < 16; }
  private heading() { this.header.content = `BEEHIVE ${this.scope === 0 ? '[Local Host]' : 'Local Host'} | ${this.scope === 1 ? '[Agents]' : 'Agents'}\nOwner · ${clean(this.owner)}`; }
  setOwner(publicSuffix?: string) { this.owner = publicSuffix ? `${publicSuffix} · signed in · key saved here` : 'signed out'; this.heading(); }
  private switchScope(scope: number) {
    if (this.modal || this.busy || this.small.visible || this.closed) return;
    this.scope = scope; this.heading(); this.onScope(scope);
  }
  private key(key: KeyEvent) {
    if (key.ctrl && (key.name === 'q' || key.name === 'c')) { key.preventDefault(); this.close(); return; }
    if (this.small.visible) { key.preventDefault(); return; }
    if (this.modal) { this.modal.key(key); return; }
    if (key.name === 'left' || key.name === 'right') { key.preventDefault(); this.switchScope(1 - this.scope); }
    if (key.name === 'tab') { key.preventDefault(); this.focusIndex = (this.focusIndex + (key.shift ? 2 : 1)) % 3; [this.list, this.scroll, this.actions][this.focusIndex]!.focus(); }
    if (key.name === 'f1') { key.preventDefault(); this.notice('Left/Right: Local Host / Agents · Tab: panes · arrows/PgDn/wheel: scroll · Enter: action · Esc: cancel form · Ctrl-Q: quit, never Stop'); }
  }
  private async run(index: number) {
    const action = this.commands[index];
    if (!action || this.busy || this.modal || this.small.visible || this.closed) return;
    this.busy = true; this.notice('Working… Esc cancels an open form. Quit is not Stop.');
    try { await action.run(); }
    catch { this.notice('Action failed. State may be unchanged or outcome unknown; inspect the operation before retrying.'); }
    finally { this.busy = false; }
  }
  show(rows: ManagerRow[], actions: ManagerAction[], selected?: string) {
    if (this.closed) return;
    const prior = selected ?? this.rows[this.list.getSelectedIndex()]?.id;
    this.rows = rows; this.commands = actions;
    this.list.options = rows.map(row => ({ name: clean(row.label), description: '', value: row.id }));
    const index = Math.max(0, rows.findIndex(row => row.id === prior));
    this.list.setSelectedIndex(index); this.detail.content = clean(rows[index]?.detail ?? 'No items.');
    this.actions.options = actions.map(action => ({ name: clean(action.label), description: '' }));
  }
  notice(text: string) { if (!this.closed) this.status.content = clean(text); }
  unavailable(feature: string) { this.notice(`${feature}: Not available in this build. No request made. Local Host and existing host observations remain separate.`); }

  /** Secret bytes never enter an Input/Textarea, history, selection or text buffer.
   * Single-use modal storage is cleared on every settlement, including quit. */
  input(label: string, value = '', secret = false, multiline = false): Promise<string | undefined> {
    if (this.closed || this.modal) return Promise.resolve(undefined);
    return new Promise(resolve => {
      const box = new BoxRenderable(this.renderer, { position: 'absolute', top: 2, left: 0, width: '100%', height: '80%', border: true, backgroundColor: '#172d3b', flexDirection: 'column', padding: 1, title: secret ? 'Hidden key input' : 'Edit' });
      this.renderer.root.add(box);
      box.add(new TextRenderable(this.renderer, { content: clean(label), flexShrink: 0 }));
      let hidden = secret ? value : '';
      const entry = secret ? undefined : multiline
        ? new TextareaRenderable(this.renderer, { initialValue: value, flexGrow: 1, width: '100%' })
        : new InputRenderable(this.renderer, { value, maxLength: 8192, width: '100%' });
      if (entry) { box.add(entry); entry.focus(); }
      else { this.list.blur(); this.actions.blur(); this.scroll.blur(); box.add(new TextRenderable(this.renderer, { content: 'Input hidden — no characters or length displayed.', height: 2 })); }
      box.add(new TextRenderable(this.renderer, { content: multiline ? 'Ctrl-S save · Esc cancel (unsaved text discarded)' : 'Enter accept · Esc cancel', height: 2 }));
      let settled = false;
      const finish = (answer?: string) => {
        if (settled) return; settled = true; hidden = '';
        if (entry instanceof InputRenderable) entry.value = '';
        else entry?.setText('');
        this.modal = undefined; box.destroyRecursively();
        if (!this.closed) [this.list, this.scroll, this.actions][this.focusIndex]!.focus();
        resolve(answer);
      };
      this.modal = {
        cancel: () => finish(),
        paste: text => { if (secret && hidden.length + text.length <= 4096) hidden += text.replace(/[\r\n]/g, ''); },
        key: key => {
          if (key.name === 'escape') { key.preventDefault(); finish(); return; }
          if ((!multiline && key.name === 'return') || (multiline && key.ctrl && key.name === 's')) {
            key.preventDefault(); finish(secret ? hidden : entry instanceof InputRenderable ? entry.value : entry?.plainText); return;
          }
          if (!secret) return;
          key.preventDefault();
          if (key.name === 'backspace') hidden = hidden.slice(0, -1);
          else if (key.ctrl && key.name === 'u') hidden = '';
          else if (!key.ctrl && !key.meta && key.sequence.length === 1 && key.sequence >= ' ' && hidden.length < 4096) hidden += key.sequence;
        },
      };
    });
  }
  async confirm(label: string) { return await this.input(`${label}\nType yes to confirm. Empty Enter is Cancel.`) === 'yes'; }
  close() {
    if (this.closed) return; this.closed = true; this.modal?.cancel(); this.renderer.destroy(); this.finish();
  }
}
