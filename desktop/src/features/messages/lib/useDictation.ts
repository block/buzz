import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Selection } from "@tiptap/pm/state";
import type { Editor } from "@tiptap/react";
import { toast } from "sonner";

import {
  setupAudioWorklet,
  type AudioWorkletHandle,
} from "@/features/huddle/lib/audioWorklet";
import { transcriptDiff } from "./transcriptDiff";

export type DictationStatus = "idle" | "starting" | "recording";

/** How long after stop() this hook still claims incoming transcripts.
 *  The Rust worker flushes the trailing phrase after the key is released;
 *  decoding can take a couple of seconds. */
// ponytail: time-based ownership window; replace with a session id threaded
// through the Rust event if two composers ever dictate back-to-back within 15s.
const OWNERSHIP_DECAY_MS = 15_000;

/** Hold plain Space this long to start dictation. A quick tap still types a
 *  space as normal — only key-repeats during the hold are suppressed. */
const SPACE_HOLD_MS = 1500;

type Session = { handle: AudioWorkletHandle; stream: MediaStream };

/** Payload of the Rust `dictation-transcript` event. Partials re-decode the
 *  in-progress phrase (~every 300 ms) and replace each other; a final commits
 *  the phrase. */
type TranscriptPayload = { text: string; final: boolean };

/** The dictation stream's place in the doc. `anchor` is where the current
 *  phrase starts; `partialText` is the phrase text currently shown there
 *  (empty right after a phrase commits). Kept to verify the region is still
 *  ours before replacing it. */
type StreamState = { anchor: number; partialText: string };

/**
 * Hold-to-talk streaming dictation for the message composer.
 *
 * start(): starts the Rust STT session (`start_dictation`), opens the mic via
 * getUserMedia, and streams PCM through the existing AudioWorklet to
 * `push_dictation_pcm`. The Rust pipeline emits `dictation-transcript` events
 * while the user talks: partials replace the in-progress phrase in place (so
 * words appear as they're spoken), finals commit it. stop(): releases the mic;
 * Rust flushes the trailing phrase, which arrives shortly after.
 *
 * Triggers: hold plain Space for SPACE_HOLD_MS (quick tap still types a
 * space), hold ⌃Space (instant start), or the toolbar mic button (click
 * toggle).
 *
 * Multiple composers may mount this hook — only the one that started the
 * session inserts the transcript (ownsRef), and Rust rejects a second
 * concurrent session.
 */
export function useDictation(editor: Editor | null) {
  const [status, setStatus] = React.useState<DictationStatus>("idle");
  const sessionRef = React.useRef<Session | null>(null);
  const streamRef = React.useRef<StreamState | null>(null);
  // Incremented on every start/stop — detects "released before start finished".
  const genRef = React.useRef(0);
  const ownsRef = React.useRef(false);
  const ownershipTimerRef = React.useRef<number | null>(null);
  // True while dictation was started by a held key (Space or ⌃Space).
  const keyHeldRef = React.useRef(false);
  // Pending "Space held long enough?" timer.
  const spaceTimerRef = React.useRef<number | null>(null);
  const editorRef = React.useRef(editor);
  editorRef.current = editor;

  // Stream transcripts from the Rust pipeline into the editor: each partial
  // replaces the previous partial of the same phrase in place, and the final
  // commits it (with a trailing space) so the next phrase starts fresh.
  //
  // The anchor is read from the live selection ONCE (first transcript of the
  // session); after that, phrases flow strictly in sequence from it. Never
  // re-read the selection or call focus() per event — chasing DOM focus and
  // selection every ~300 ms made the caret flick around while talking.
  React.useEffect(() => {
    const unlisten = listen<TranscriptPayload>(
      "dictation-transcript",
      (event) => {
        if (!ownsRef.current) return;
        const editor = editorRef.current;
        if (!editor) return;
        const text = event.payload.text.trim();
        if (!text) return;
        const insert = event.payload.final ? `${text} ` : text;

        const doc = editor.state.doc;
        let stream = streamRef.current;
        if (stream) {
          const end = stream.anchor + stream.partialText.length;
          const intact =
            end <= doc.content.size &&
            doc.textBetween(stream.anchor, end) === stream.partialText;
          if (!intact) {
            // The user edited around the stream (possible between key release
            // and the trailing final) — resume at the end of the doc.
            stream = { anchor: Selection.atEnd(doc).from, partialText: "" };
          }
        } else {
          // First transcript of the session: anchor at the cursor.
          stream = { anchor: editor.state.selection.from, partialText: "" };
        }

        // Only rewrite the part of the phrase that actually changed. Most
        // re-decodes append or are identical — a full delete+reinsert every
        // ~300 ms churned the DOM and selection even when nothing changed,
        // which is what made the caret visibly bounce while talking.
        const { anchor } = stream;
        const diff = transcriptDiff(stream.partialText, insert);
        if (diff.deleteLen > 0 || diff.insert.length > 0) {
          const from = anchor + diff.keep;
          const chain = editor.chain();
          if (diff.deleteLen > 0) {
            chain.deleteRange({ from, to: from + diff.deleteLen });
          }
          if (diff.insert.length > 0) {
            // Insert as a plain text node — never parse transcript as HTML.
            chain.insertContentAt(from, { type: "text", text: diff.insert });
          }
          // Pin the caret to the end of the dictated text.
          chain.setTextSelection(anchor + insert.length).run();
        }
        streamRef.current = event.payload.final
          ? { anchor: anchor + insert.length, partialText: "" }
          : { anchor, partialText: insert };
        // Hide the blinking caret while words are streaming in — it reads as
        // noise next to live-updating text (chl request). It comes back at
        // each pause (phrase commit) and when dictation stops.
        editor.view.dom.style.caretColor = event.payload.final
          ? ""
          : "transparent";
      },
    );
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  const teardownSession = React.useCallback(() => {
    const session = sessionRef.current;
    sessionRef.current = null;
    if (session) {
      session.handle.stop();
      for (const track of session.stream.getTracks()) {
        track.stop();
      }
    }
  }, []);

  const stop = React.useCallback(() => {
    genRef.current += 1;
    teardownSession();
    setStatus("idle");
    // Restore the caret even if no final ever arrives for the last phrase.
    const dom = editorRef.current?.view.dom;
    if (dom) dom.style.caretColor = "";
    void invoke("stop_dictation").catch(() => {
      /* nothing to stop */
    });
    // Keep claiming transcripts briefly — the flush arrives after stop.
    if (ownershipTimerRef.current !== null) {
      window.clearTimeout(ownershipTimerRef.current);
    }
    ownershipTimerRef.current = window.setTimeout(() => {
      ownsRef.current = false;
      ownershipTimerRef.current = null;
    }, OWNERSHIP_DECAY_MS);
  }, [teardownSession]);

  const start = React.useCallback(async (): Promise<void> => {
    if (sessionRef.current) return;
    const gen = ++genRef.current;
    // A stream left over from a previous session (final never arrived, or the
    // doc changed) must not swallow this session's first phrase.
    streamRef.current = null;
    setStatus("starting");
    // Tracks whether the Rust session was started, so cleanup on a later
    // failure never kills a session owned by another composer.
    let rustSessionStarted = false;
    try {
      // Rust first: fails fast if the model is missing or a session is live.
      await invoke("start_dictation");
      rustSessionStarted = true;
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const track = stream.getAudioTracks()[0];
      if (!track) {
        for (const t of stream.getTracks()) t.stop();
        throw new Error("No microphone input available");
      }
      const handle = await setupAudioWorklet(track, true, "push_dictation_pcm");
      if (gen !== genRef.current) {
        // Released (or restarted) while we were setting up — undo.
        handle.stop();
        for (const t of stream.getTracks()) t.stop();
        return;
      }
      sessionRef.current = { handle, stream };
      ownsRef.current = true;
      if (ownershipTimerRef.current !== null) {
        window.clearTimeout(ownershipTimerRef.current);
        ownershipTimerRef.current = null;
      }
      setStatus("recording");
    } catch (error) {
      if (gen === genRef.current) {
        setStatus("idle");
        if (rustSessionStarted) {
          void invoke("stop_dictation").catch(() => {});
        }
      }
      throw error instanceof Error ? error : new Error(String(error));
    }
  }, []);

  // Release the mic if the composer unmounts mid-recording.
  React.useEffect(() => {
    return () => {
      if (sessionRef.current) {
        teardownSession();
        void invoke("stop_dictation").catch(() => {});
      }
      if (ownershipTimerRef.current !== null) {
        window.clearTimeout(ownershipTimerRef.current);
      }
      if (spaceTimerRef.current !== null) {
        window.clearTimeout(spaceTimerRef.current);
      }
    };
  }, [teardownSession]);

  const startWithToast = React.useCallback(() => {
    void start().catch((error: Error) => {
      toast.error(error.message || "Could not start dictation");
    });
  }, [start]);

  /** Mic button click — start when idle, stop otherwise. */
  const statusRef = React.useRef(status);
  statusRef.current = status;
  const toggle = React.useCallback(() => {
    if (statusRef.current === "idle") {
      startWithToast();
    } else {
      stop();
    }
  }, [startWithToast, stop]);

  // Hold Space (or ⌃Space) = push-to-talk. Keydown comes from the composer
  // (so only the focused composer starts); the release can land anywhere — or
  // nowhere, on focus loss — so keyup/blur are handled on window.
  React.useEffect(() => {
    const release = () => {
      if (spaceTimerRef.current !== null) {
        window.clearTimeout(spaceTimerRef.current);
        spaceTimerRef.current = null;
      }
      if (!keyHeldRef.current) return;
      keyHeldRef.current = false;
      stop();
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (event.code === "Space" || event.key === "Control") release();
    };
    // Capture phase: while a hold is pending or live, swallow Space
    // key-repeats before they reach ANY element. If focus drifts off the
    // composer mid-hold, leaked repeats would click focused buttons, scroll
    // the page, or type spaces into the stream.
    const onKeyDownCapture = (event: KeyboardEvent) => {
      if (
        event.code === "Space" &&
        event.repeat &&
        (keyHeldRef.current || spaceTimerRef.current !== null)
      ) {
        event.preventDefault();
        event.stopPropagation();
      }
    };
    window.addEventListener("keydown", onKeyDownCapture, true);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", release);
    return () => {
      window.removeEventListener("keydown", onKeyDownCapture, true);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", release);
    };
  }, [stop]);

  /** Returns true when the event was consumed. Handles both triggers:
   *  ⌃Space (instant) and plain Space held for SPACE_HOLD_MS. */
  const handleComposerKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLElement>): boolean => {
      if (event.code !== "Space" || event.metaKey || event.altKey) {
        return false;
      }

      // ⌃Space — instant hold-to-talk.
      if (event.ctrlKey) {
        event.preventDefault();
        if (!event.repeat && !keyHeldRef.current) {
          keyHeldRef.current = true;
          startWithToast();
        }
        return true;
      }
      if (event.shiftKey) return false;

      // Plain Space — a quick tap types a space as normal; holding for
      // SPACE_HOLD_MS starts dictation. Key-repeats are swallowed while the
      // hold is pending or active so a hold doesn't spray spaces.
      if (event.repeat) {
        if (spaceTimerRef.current !== null || keyHeldRef.current) {
          event.preventDefault();
          return true;
        }
        return false;
      }
      if (spaceTimerRef.current === null && !keyHeldRef.current) {
        spaceTimerRef.current = window.setTimeout(() => {
          spaceTimerRef.current = null;
          keyHeldRef.current = true;
          // The keydown typed a space before the hold was recognised — the
          // hold means "talk", not "space", so take it back.
          const ed = editorRef.current;
          if (ed) {
            const { from } = ed.state.selection;
            if (from > 0 && ed.state.doc.textBetween(from - 1, from) === " ") {
              ed.commands.deleteRange({ from: from - 1, to: from });
            }
          }
          startWithToast();
        }, SPACE_HOLD_MS);
      }
      return false; // let the space type normally
    },
    [startWithToast],
  );

  return { status, start, stop, toggle, handleComposerKeyDown };
}
