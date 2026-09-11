import { RGBA, BoxRenderable, InputRenderable, ScrollBoxRenderable, SelectRenderable, TextRenderable, TextareaRenderable, type CliRenderer, type KeyEvent } from '@opentui/core';
import { stripVTControlCharacters } from 'node:util';

export type ManagerRow = { id: string; label: string; detail: string; evidence?: string };
export type ManagerAction = { label: string; disabled?: string; run: () => void | Promise<void> };
const fg = RGBA.defaultForeground(), bg = RGBA.defaultBackground();
const clean = (value: string) => stripVTControlCharacters(value).replace(/[\x00-\x08\x0b-\x1f\x7f]/g, '');

/** Presentation only: never imports credentials, configuration, transport or host code.
 * Bun owns this renderer; the manager/service interpreter can remain Node. */
export class OpenTuiScreen {
  private readonly root: BoxRenderable;
  private readonly header: TextRenderable;
  private readonly list: SelectRenderable;
  private readonly listFrame: BoxRenderable;
  private readonly detail: TextRenderable;
  private readonly scroll: ScrollBoxRenderable;
  private readonly actions: SelectRenderable;
  private readonly status: TextRenderable;
  private readonly footer: TextRenderable;
  private readonly small: TextRenderable;
  private rows: ManagerRow[] = [];
  private commands: ManagerAction[] = [];
  private focusIndex = 0;
  private scope = 0;
  private owner = 'signed out';
  private busy = false;
  private closed = false;
  private overlay?: BoxRenderable;
  private compactOverlay = false;
  private modal?: { cancel: () => void; key: (key: KeyEvent) => void; paste: (text: string) => void };
  private finish!: () => void;
  readonly done = new Promise<void>(resolve => { this.finish = resolve; });
  onScope: (index: number) => void = () => {};
  onSelect: (id: string) => void = () => {};

  /** Persistent ordinary-key legend, separate from status. Never function-key first.
   * The compact form keeps every affordance named at the 40-column narrow floor. */
  private static readonly hintsWide = '? Help · a Actions · i Inspect · o Status · q Quit';
  private static readonly hintsNarrow = '? a Actions i Inspect o Status q Quit';

  constructor(readonly renderer: CliRenderer) {
    this.root = new BoxRenderable(renderer, { width: '100%', height: '100%', flexDirection: 'column', backgroundColor: bg });
    renderer.root.add(this.root);
    this.header = new TextRenderable(renderer, { fg, height: 2, onMouseDown: event => { if (event.y !== 0) return; const end = this.scope === 0 ? 20 : 18; if (event.x >= 8 && event.x < end) this.switchScope(0); else if (event.x >= end + 3 && event.x < end + 3 + (this.scope === 1 ? 8 : 6)) this.switchScope(1); } });
    this.root.add(this.header);
    const body = new BoxRenderable(renderer, { flexGrow: 1, flexDirection: 'row', minHeight: 1 });
    this.root.add(body);
    this.listFrame = new BoxRenderable(renderer, { width: 28, height: '100%', border: true, title: '[List]' }); body.add(this.listFrame);
    this.list = new SelectRenderable(renderer, { backgroundColor: bg, textColor: fg, focusedBackgroundColor: bg, focusedTextColor: fg, selectedBackgroundColor: fg, selectedTextColor: bg, width: '100%', height: '100%', showDescription: false, showScrollIndicator: true, showSelectionIndicator: true, onMouseScroll: event => { if (event.scroll?.direction === 'up') this.list.moveUp(); else if (event.scroll?.direction === 'down') this.list.moveDown(); }, onMouseDown: event => {
      this.focusIndex = 0; this.list.focus();
      const delta = event.y - this.list.y;
      // Pinned OpenTUI does not expose its scrolling offset. Never map a click
      // to an unrelated row; keyboard navigation remains available for long lists.
      if (this.rows.length <= this.list.height && delta >= 0) this.list.setSelectedIndex(Math.min(this.rows.length - 1, delta));
      else this.notice('Use arrows to select an item in this list. Mouse selection is unavailable.');
    } });
    this.listFrame.add(this.list);
    this.scroll = new ScrollBoxRenderable(renderer, { flexGrow: 1, height: '100%', border: true, title: 'Details', scrollY: true, onMouseDown: () => { this.focusIndex = 1; this.scroll.focus(); } });
    body.add(this.scroll);
    this.detail = new TextRenderable(renderer, { fg, width: '100%', content: 'No items.', selectable: false });
    this.scroll.add(this.detail);
    this.actions = new SelectRenderable(renderer, { backgroundColor: bg, textColor: fg, focusedBackgroundColor: bg, focusedTextColor: fg, selectedBackgroundColor: fg, selectedTextColor: bg, height: 2, showDescription: false, showSelectionIndicator: true, options: [{name: 'Actions (a)', description: ''}, {name: 'Inspect (i)', description: ''}], onMouseDown: event => { this.focusIndex = 2; this.actions.focus(); this.actions.setSelectedIndex(Math.max(0, Math.min(1, event.y - this.actions.y))); this.actions.selectCurrent(); } });
    this.root.add(this.actions);
    this.status = new TextRenderable(renderer, { fg, height: 2, onMouseDown: () => this.outcome() });
    this.root.add(this.status);
    this.footer = new TextRenderable(renderer, { fg, height: 1, content: OpenTuiScreen.hintsWide });
    this.root.add(this.footer);
    this.small = new TextRenderable(renderer, { fg, position: 'absolute', top: 0, left: 0, width: '100%', height: '100%', bg, content: 'Resize to at least 40 × 16. Ctrl-Q quits. Existing operations can continue.', visible: false });
    renderer.root.add(this.small);
    this.list.on('selectionChanged', (index: number) => {
      const row = this.rows[index]; this.detail.content = clean(row?.detail ?? 'No items.'); this.scroll.scrollTo(0);
      this.focusStyle(); if (row) this.onSelect(row.id);
    });
    this.actions.on('itemSelected', (index: number) => { if (index === 0) this.drawer(); else this.inspect(); });
    this.list.on('itemSelected', () => { if (this.narrow) { this.drilled = true; this.resize(); this.focusIndex = 1; this.scroll.focus(); } });
    renderer.keyInput.on('keypress', key => this.key(key));
    renderer.keyInput.on('paste', event => {
      if (!this.modal) return;
      this.modal.paste(new TextDecoder().decode(event.bytes));
      // Ordinary editor paste is handled by its focused renderable.
    });
    renderer.on('resize', () => this.resize());
    renderer.on('destroy', () => this.close());
    for (const widget of [this.list, this.scroll, this.actions]) { widget.on('focused', () => this.focusStyle()); widget.on('blurred', () => this.focusStyle()); }
    this.list.focus(); this.heading(); this.resize();
  }

  private focusStyle() {
    this.listFrame.title = `${this.list.focused ? '[List]' : 'List'} · ${this.list.getSelectedIndex() + 1}/${this.rows.length}`;
    this.scroll.title = `${this.scroll.focused ? '[Details]' : 'Details'}${this.narrow ? ' · Esc back' : ''}`;
    for (const widget of [this.list, this.actions]) { widget.selectedBackgroundColor = widget.focused ? fg : bg; widget.selectedTextColor = widget.focused ? bg : fg; }
  }
  private narrow = false;
  private drilled = false;
  private pending = false;
  private lastNotice = '';
  onCancel: () => void = () => {};
  setPending(value: boolean) { this.pending = value; }
  private resize() {
    if (this.overlay && this.modal) { this.overlay.width = Math.min(72, this.renderer.width - 4); this.overlay.left = Math.floor((this.renderer.width - Math.min(72, this.renderer.width - 4)) / 2); this.overlay.height = this.compactOverlay ? Math.min(14, this.renderer.height - 4) : this.renderer.height - 4; }
    this.small.visible = this.renderer.width < 40 || this.renderer.height < 16;
    this.narrow = this.renderer.width < 88 || this.renderer.height < 24;
    this.listFrame.visible = !this.narrow || !this.drilled;
    this.scroll.visible = !this.narrow || this.drilled;
    this.listFrame.width = this.narrow ? '100%' : Math.min(32, Math.max(24, Math.floor(this.renderer.width * .28)));
    this.scroll.title = this.narrow ? '[Details] · Esc back' : 'Details';
    this.footer.content = this.renderer.width >= 52 ? OpenTuiScreen.hintsWide : OpenTuiScreen.hintsNarrow;
  }
  private heading() { this.header.content = `BEEHIVE ${this.scope === 0 ? '[Local Host]' : 'Local Host'} | ${this.scope === 1 ? '[Agents]' : 'Agents'}\n${this.scope === 0 ? 'This computer · no owner sign-in needed' : 'Owner · ' + clean(this.owner)}`; }
  setOwner(publicSuffix?: string) { this.owner = publicSuffix ? `${publicSuffix} · signed in · key saved here` : 'signed out'; this.heading(); }
  private switchScope(scope: number) {
    if (this.modal || this.busy || this.small.visible || this.closed) return;
    this.scope = scope; this.drilled = false; this.resize(); this.heading(); this.onScope(scope);
  }
  private key(key: KeyEvent) {
    if (key.ctrl && (key.name === 'q' || key.name === 'c')) { key.preventDefault(); this.close(); return; }
    if (this.small.visible) { key.preventDefault(); return; }
    if (key.name === 'escape' && this.pending) { key.preventDefault(); this.onCancel(); return; }
    if (this.modal) { this.modal.key(key); return; }
    if (key.name === 'escape' && this.drilled) { key.preventDefault(); this.drilled = false; this.resize(); this.focusIndex = 0; this.list.focus(); }
    // Ordinary keys are the primary map. Every editable field is a modal handled
    // above, so these never intercept text in public, secret or multiline inputs.
    if (!key.ctrl && !key.meta) {
      if (key.name === '?') { key.preventDefault(); this.help(); return; }
      if (key.name === 'a') { key.preventDefault(); this.drawer(); return; }
      if (key.name === 'i') { key.preventDefault(); this.inspect(); return; }
      if (key.name === 'o') { key.preventDefault(); this.outcome(); return; }
      if (key.name === 'q') { key.preventDefault(); this.close(); return; }
    }
    // Legacy compatibility aliases only; the advertised map above uses ordinary keys.
    if (key.name === 'f2') { key.preventDefault(); this.drawer(); return; }
    if (key.name === 'f3') { key.preventDefault(); this.inspect(); return; }
    if (key.name === 'f4') { key.preventDefault(); this.outcome(); return; }
    if (this.focusIndex === 0 && ['home','end','pageup','pagedown'].includes(key.name)) { key.preventDefault(); this.navigate(this.list, key.name); }
    if (key.name === 'left' || key.name === 'right') { key.preventDefault(); this.switchScope(1 - this.scope); }
    if (key.name === 'tab') { key.preventDefault(); this.focusIndex = (this.focusIndex + (key.shift ? 2 : 1)) % 3; if (this.narrow && this.focusIndex === (this.drilled ? 0 : 1)) this.focusIndex = (this.focusIndex + (key.shift ? 2 : 1)) % 3; [this.list, this.scroll, this.actions][this.focusIndex]!.focus(); this.focusStyle(); }
    if (key.name === 'f1') { key.preventDefault(); this.help(); }
  }
  private async run(index: number, captured?: ManagerAction) {
    const action = captured ?? this.commands[index];
    if (!action || this.pending || this.busy || this.modal || this.small.visible || this.closed) return;
    if (action.disabled) { this.notice(action.disabled); return; }
    this.busy = true; this.notice('Working… Esc cancels an open form. Quitting does not stop agents.');
    try { await action.run(); }
    catch { this.notice('Action failed. The result is unknown. Inspect the operation before you try again.'); }
    finally { this.busy = false; }
  }
  show(rows: ManagerRow[], actions: ManagerAction[], selected?: string) {
    if (this.closed) return;
    const prior = selected ?? this.rows[this.list.getSelectedIndex()]?.id;
    this.rows = rows; this.commands = actions;
    const options = rows.map(row => ({ name: clean(row.label), description: '', value: row.id }));
    if (JSON.stringify(this.list.options) !== JSON.stringify(options)) this.list.options = options;
    const index = Math.max(0, rows.findIndex(row => row.id === prior));
    this.list.setSelectedIndex(index); this.detail.content = clean(rows[index]?.detail ?? 'No items.'); this.focusStyle();

  }
  notice(text: string) { if (!this.closed) { this.lastNotice = clean(text); this.status.content = this.lastNotice; } }
  unavailable(feature: string) { this.notice(`${feature} is unavailable in this build. No request was sent. Local Host shows this computer. Agents shows host reports.`); }

  choose(title: string, names: string[]): Promise<string | undefined> {
    if (this.modal || !names.length) return Promise.resolve(undefined);
    return new Promise(resolve => {
      const box = new BoxRenderable(this.renderer, { position: 'absolute', top: 2, left: Math.floor((this.renderer.width - Math.min(72, this.renderer.width - 4)) / 2), width: Math.min(72, this.renderer.width - 4), height: this.renderer.height - 4, border: true, title: title + ' · Esc cancel', backgroundColor: bg });
      const list = new SelectRenderable(this.renderer, { backgroundColor: bg, textColor: fg, focusedBackgroundColor: bg, focusedTextColor: fg, selectedBackgroundColor: fg, selectedTextColor: bg, width: '100%', height: '100%', showDescription: false, showSelectionIndicator: true, options: names.map(name => ({ name: clean(name), description: '' })) });
      this.overlay = box; this.compactOverlay = false; this.renderer.root.add(box); box.add(list); list.focus();
      const finish = (name?: string) => { this.modal = undefined; box.destroyRecursively(); this.actions.focus(); resolve(name); };
      list.on('itemSelected', (i: number) => finish(names[i]));
      this.modal = { cancel: () => finish(), paste: () => {}, key: key => { if (key.name === 'escape') { key.preventDefault(); finish(); } else if (['home','end','pageup','pagedown'].includes(key.name)) { key.preventDefault(); this.navigate(list, key.name); } } };
    });
  }
  private navigate(list: SelectRenderable, key: string) {
    const n = list.options.length, current = list.getSelectedIndex();
    list.setSelectedIndex(Math.max(0, Math.min(n - 1, key === 'home' ? 0 : key === 'end' ? n - 1 : current + (key === 'pageup' ? -1 : 1) * Math.max(1, list.height))));
  }
  inspect() { const row = this.rows[this.list.getSelectedIndex()]; this.read('Technical details', row?.evidence ?? 'No technical details for this item.'); }
  private outcome() { this.read('Status · latest message', this.lastNotice || 'No status message yet.'); }
  private help() { this.read('Help', '? Help · a Actions · i Inspect · o Status · q Quit\nLeft/Right: switch Local Host and Agents.\nTab/Shift-Tab: switch panes.\nArrows, PgUp/PgDn, Home/End or wheel: scroll.\nEnter: open details or an action.\nEsc: cancel waiting first, then close or go back.\nq or Ctrl-Q: quit. Hosts and agents keep running.\nUse arrows and Enter in scrolled lists. Mouse selection is unavailable.\nShortcuts do not run while you edit a field.\nQuit before you use a foreground CLI command.\nRestart and publication forms are unavailable.'); }
  private read(title: string, content: string) {
    if (this.modal || this.closed) return;
    const box = new ScrollBoxRenderable(this.renderer, { position: 'absolute', top: 2, left: Math.floor((this.renderer.width - Math.min(72, this.renderer.width - 4)) / 2), width: Math.min(72, this.renderer.width - 4), height: this.renderer.height - 4, border: true, title: title + ' · Esc close', backgroundColor: bg, scrollY: true });
    this.overlay = box; this.compactOverlay = false; this.renderer.root.add(box); box.add(new TextRenderable(this.renderer, { fg, content: clean(content), width: '100%', selectable: true })); box.focus();
    const cancel = () => { this.modal = undefined; box.destroyRecursively(); [this.list, this.scroll, this.actions][this.focusIndex]!.focus(); };
    this.modal = { cancel, paste: () => {}, key: key => { if (key.name === 'escape') { key.preventDefault(); cancel(); } } };
  }
  private drawer() {
    if (this.modal || this.closed) return;
    const commands = this.commands.slice();
    const box = new BoxRenderable(this.renderer, { position: 'absolute', top: 2, left: Math.floor((this.renderer.width - Math.min(72, this.renderer.width - 4)) / 2), width: Math.min(72, this.renderer.width - 4), height: this.renderer.height - 4, border: true, title: '[Actions] · Esc close', backgroundColor: bg, flexDirection: 'column', padding: 1 });
    this.overlay = box; this.compactOverlay = false; this.renderer.root.add(box);
    box.add(new TextRenderable(this.renderer, { fg, content: clean(this.rows[this.list.getSelectedIndex()]?.label ?? 'No selection'), height: 2 }));
    const reason = new TextRenderable(this.renderer, { fg, height: 4 });
    const list = new SelectRenderable(this.renderer, { backgroundColor: bg, textColor: fg, focusedBackgroundColor: bg, focusedTextColor: fg, selectedBackgroundColor: fg, selectedTextColor: bg, flexGrow: 1, showDescription: false, showSelectionIndicator: true, showScrollIndicator: true, options: commands.map(a => ({ name: clean((a.disabled || this.pending ? '(unavailable) ' : '') + a.label), description: '' })), onMouseScroll: event => { if (event.scroll?.direction === 'up') list.moveUp(); else if (event.scroll?.direction === 'down') list.moveDown(); }, onMouseDown: () => this.notice('Use arrows, then Enter. Mouse selection is unavailable. No request was sent.') });
    box.add(list); box.add(reason); list.focus();
    const explain = () => { reason.content = this.pending ? 'Working… Esc stops waiting. Remote work or saved changes may already be complete. Inspect before you try again.' : clean(commands[list.getSelectedIndex()]?.disabled || 'Enter opens · Esc closes · arrows/PgDn scroll'); };
    explain(); list.on('selectionChanged', explain);
    const cancel = () => { this.modal = undefined; box.destroyRecursively(); [this.list, this.scroll, this.actions][this.focusIndex]!.focus(); };
    list.on('itemSelected', (i: number) => { if (this.pending || commands[i]?.disabled) { explain(); return; } cancel(); void this.run(i, commands[i]); });
    this.modal = { cancel, paste: () => {}, key: key => { if (key.name === 'escape') { key.preventDefault(); cancel(); } else if (['home','end','pageup','pagedown'].includes(key.name)) { key.preventDefault(); this.navigate(list, key.name); } } };
  }

  /** Secret bytes never enter an Input/Textarea, history, selection or text buffer.
   * Single-use modal storage is cleared on every settlement, including quit. */
  input(label: string, value = '', secret = false, multiline = false, validate?: (value: string) => string, back?: (value: string) => void): Promise<string | undefined> {
    if (this.closed || this.modal) return Promise.resolve(undefined);
    return new Promise(resolve => {
      const box = new BoxRenderable(this.renderer, { position: 'absolute', top: 2, left: Math.floor((this.renderer.width - Math.min(72, this.renderer.width - 4)) / 2), width: Math.min(72, this.renderer.width - 4), height: multiline ? this.renderer.height - 4 : Math.min(this.renderer.height - 4, 14), border: true, backgroundColor: bg, flexDirection: 'column', padding: 1, title: secret ? 'Hidden key input' : multiline ? 'Private draft · Ctrl-S saves' : clean(label.split('\n')[0]!).slice(0, 45) });
      this.overlay = box; this.compactOverlay = false; this.renderer.root.add(box);
      this.compactOverlay = !multiline;
      const prompt = new ScrollBoxRenderable(this.renderer, { width: '100%', flexGrow: multiline ? 0 : 1, height: multiline ? 3 : undefined, scrollY: true });
      prompt.add(new TextRenderable(this.renderer, { fg, content: clean(label), width: '100%' })); box.add(prompt);
      let hidden = secret ? value : '';
      const entry = secret ? undefined : multiline
        ? new TextareaRenderable(this.renderer, { initialValue: value, flexGrow: 1, width: '100%' })
        : new InputRenderable(this.renderer, { value, maxLength: 8192, width: '100%', backgroundColor: fg, textColor: bg, focusedBackgroundColor: fg, focusedTextColor: bg });
      if (entry) { const field = new BoxRenderable(this.renderer, { border: !multiline, height: multiline ? undefined : 3, flexShrink: 0, flexGrow: multiline ? 1 : 0, width: '100%' }); field.add(entry); box.add(field); entry.focus(); }
      else { this.list.blur(); this.actions.blur(); this.scroll.blur(); box.add(new TextRenderable(this.renderer, { fg, content: 'Your key is hidden. Its length is not shown.', height: 2 })); }
      box.add(new TextRenderable(this.renderer, { fg, content: multiline ? 'Ctrl-S save · Esc cancel (unsaved text discarded)' : back ? 'Enter next · Shift-Tab back · Esc cancel' : 'Enter continue · Esc cancel · Ctrl-↑/↓ scroll', height: 2 }));
      const error = new TextRenderable(this.renderer, { fg, height: 2, flexShrink: 0 }); box.add(error);
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
          if (key.ctrl && ['up','down'].includes(key.name)) { key.preventDefault(); prompt.scrollBy(key.name === 'up' ? -3 : 3); return; }
          if (back && !secret && key.name === 'tab' && key.shift) { key.preventDefault(); back(entry instanceof InputRenderable ? entry.value : entry?.plainText ?? ''); finish('\0back'); return; }
          if (key.name === 'escape') { key.preventDefault(); finish(); return; }
          if ((!multiline && key.name === 'return') || (multiline && key.ctrl && key.name === 's')) {
            key.preventDefault(); const answer = secret ? hidden : entry instanceof InputRenderable ? entry.value : entry?.plainText ?? ''; const invalid = validate?.(answer); if (invalid) { error.content = secret ? 'Invalid key. Enter the key again.' : clean(invalid); if (secret) hidden = ''; return; } finish(answer); return;
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
  async confirm(label: string) { return await this.input(`${label}\nType yes to confirm. Press Enter with no text to cancel.`) === 'yes'; }
  close() {
    if (this.closed) return; this.closed = true; this.modal?.cancel(); this.renderer.destroy(); this.finish();
  }
}
