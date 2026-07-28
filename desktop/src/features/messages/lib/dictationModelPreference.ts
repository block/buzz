import * as React from "react";

/**
 * User preference for the dictation speech-to-text model, as a model id from
 * the Rust `STT_MODELS` registry (e.g. "parakeet-en", "parakeet-v3").
 *
 * `null` means no explicit choice: dictation uses the model the app selected
 * at startup (BUZZ_STT_MODEL override / system locale / English default).
 * The preference is passed to `start_dictation`, which falls back to the
 * startup-selected model when the preferred one isn't downloaded yet.
 *
 * Persisted in localStorage — a device-level preference, like the thread
 * layout in threadViewModePreference.ts.
 */
export type DictationModelPreference = string | null;

const STORAGE_KEY = "buzz.dictation.model";

const listeners = new Set<() => void>();

let preference: DictationModelPreference = readStoredPreference();

function readStoredPreference(): DictationModelPreference {
  try {
    return globalThis.localStorage?.getItem(STORAGE_KEY) ?? null;
  } catch {
    return null;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): DictationModelPreference {
  return preference;
}

function getServerSnapshot(): DictationModelPreference {
  return null;
}

/** Read the persisted dictation model preference outside of React. */
export function getDictationModelPreference(): DictationModelPreference {
  return preference;
}

/** Update the dictation model preference and notify subscribed components. */
export function setDictationModelPreference(
  model: DictationModelPreference,
): void {
  preference = model;

  try {
    if (model === null) {
      globalThis.localStorage?.removeItem(STORAGE_KEY);
    } else {
      globalThis.localStorage?.setItem(STORAGE_KEY, model);
    }
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

/** The dictation model preference (null = app default). */
export function useDictationModelPreference(): DictationModelPreference {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
