/**
 * Tracks which composer instance should handle global dictation shortcuts.
 *
 * When multiple composers are mounted (e.g. channel + thread), only the
 * most recently focused one should respond to ⌘D. Each composer registers
 * on focus and the global shortcut only dispatches to the active instance.
 */

let activeInstanceId: string | null = null;

export function setActiveDictationComposer(id: string): void {
  activeInstanceId = id;
}

export function clearActiveDictationComposer(id: string): void {
  if (activeInstanceId === id) {
    activeInstanceId = null;
  }
}

export function isActiveDictationComposer(id: string): boolean {
  return activeInstanceId === id;
}
