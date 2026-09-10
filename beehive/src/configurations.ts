import { fields, object, text } from './protocol.ts';
import { selection, type Selection } from './handoff.ts';

/** Host/agent-scoped named candidates. Actual runs retain their own immutable copies. */
export type Configurations = Record<string, Selection>;
const nameOf = (value: unknown) => {
  const name = text(value, 64);
  if (!/^[A-Za-z0-9][A-Za-z0-9 _.-]*$/.test(name) || ['__proto__', 'constructor', 'prototype'].includes(name)) throw Error('Invalid configuration name');
  return name;
};
/** Legacy selected-next is projected, not rewritten or invented as historical evidence. */
export function configurations(saved: Configurations | undefined, selected: Selection): Configurations {
  const entries = saved ?? { default: { ...selected, configuration: { name: 'default', revision: 1 } } };
  const keys = Object.keys(object(entries));
  if (!keys.length || keys.length > 32 || Buffer.byteLength(JSON.stringify(entries)) > 8000) throw Error('Invalid configuration inventory');
  for (const key of keys) {
    nameOf(key); const value = selection(entries[key]);
    if (value.configuration?.name !== key) throw Error('Invalid configuration binding');
  }
  return structuredClone(entries);
}
/** Pure bounded candidate transaction; caller validates policy and atomically persists with receipt. */
export function configure(saved: Configurations | undefined, selected: Selection, body: Record<string, unknown>, revision: number) {
  const entries = configurations(saved, selected);
  let next = selected;
  if (body.configurationAction !== undefined) {
    const action = text(body.configurationAction); const name = nameOf(body.name);
    fields(body, ['configurationAction', 'name', ...(action === 'rename' ? ['newName'] : [])]);
    if (action === 'create') {
      if (Object.hasOwn(entries, name) || Object.keys(entries).length >= 32) throw Error('Configuration exists or limit reached');
      next = { ...selected, configuration: { name, revision } }; entries[name] = next;
    } else {
      if (!Object.hasOwn(entries, name)) throw Error('Unknown configuration');
      if (action === 'select') next = entries[name]!;
      else if (action === 'remove') {
        if (name === (selected.configuration?.name ?? 'default')) throw Error('Select another configuration before removal');
        delete entries[name];
      } else if (action === 'rename') {
        const newName = nameOf(body.newName);
        if (Object.hasOwn(entries, newName)) throw Error('Configuration exists');
        const renamed = { ...entries[name]!, configuration: { name: newName, revision } };
        entries[newName] = renamed; delete entries[name];
        if (name === (selected.configuration?.name ?? 'default')) next = renamed;
      } else throw Error('Unknown configuration action');
    }
  } else {
    const value = selection(body);
    const name = selected.configuration?.name ?? 'default';
    next = { ...value, configuration: { name, revision } };
    entries[name] = next;
  }
  if (Buffer.byteLength(JSON.stringify(entries)) > 8000) throw Error('Configuration inventory size limit');
  return { entries, selected: structuredClone(next) };
}

/** Materialize an already validated/prepared Move snapshot without altering any run
 * history. Called before preparation and in the atomic assignment acceptance write. */
export function materializeMove(saved: Configurations | undefined, selected: Selection, effective: Selection) {
  const entries = configurations(saved, selected);
  const next = selection(effective);
  if (next.configuration) entries[nameOf(next.configuration.name)] = next;
  return { entries: configurations(entries, next), selected: next };
}
