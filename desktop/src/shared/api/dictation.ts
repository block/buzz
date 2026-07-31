import { invokeTauri } from "@/shared/api/tauri";

/**
 * Composer dictation — thin wrappers over the Rust commands in
 * `src-tauri/src/commands/dictation.rs`.
 *
 * Transcription runs fully on-device via the same sherpa-onnx Parakeet model
 * the huddle STT pipeline uses. No network, no API key.
 */

/**
 * Whether the speech model has finished downloading.
 *
 * The model is fetched in the background on app launch, so this is false for
 * the first few minutes of a fresh install.
 */
export async function isDictationAvailable(): Promise<boolean> {
  return invokeTauri<boolean>("is_dictation_available");
}

/** Begin a dictation session. Tearing down any prior session is handled in Rust. */
export async function startDictation(): Promise<void> {
  return invokeTauri<void>("start_dictation");
}

/** End the dictation session and release the STT worker thread. */
export async function stopDictation(): Promise<void> {
  return invokeTauri<void>("stop_dictation");
}
