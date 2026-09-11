import * as ui from "./ui.mjs";
import { openVoice } from "./voice.mjs";
const $ = (id) => document.getElementById(id);
// The launch fragment stays in this tab for refresh; it is never sent in HTTP requests.
const token = location.hash.slice(1);
let voice,
  connecting,
  busy = false,
  savedFocus,
  received = 0,
  finished = false,
  measured = false,
  outputItem;
window.realtimeEvidence = [];
const approval = $("tool-approval");
// Approval is a visible choice for this page, never a saved default.
approval.value = "ask";
approval.onchange = () => {
  $("approval-note").hidden = approval.value !== "auto";
  approval
    .closest("label")
    .classList.toggle("automatic", approval.value === "auto");
};
function permission(tool, decide) {
  if (tool && approval.value === "auto") {
    $("permission").hidden = true;
    ui.note(`Automatically approved ${tool.title || "tool call"}.`);
    decide("allow_once").catch(error);
    return;
  }
  $("permission").hidden = !tool;
  if (!tool) {
    savedFocus?.focus();
    savedFocus = null;
    return;
  }
  savedFocus = document.activeElement;
  $("tool").textContent = JSON.stringify(tool, null, 2);
  $("allow").onclick = () => decide("allow_once").catch(error);
  $("deny").onclick = () => decide("reject_once").catch(error);
  $("deny").focus();
}
$("permission").onkeydown = (e) => {
  if (e.key === "Escape") {
    $("deny").click();
    e.preventDefault();
  }
  if (e.key === "Tab") {
    e.preventDefault();
    (document.activeElement === $("deny") ? $("allow") : $("deny")).focus();
  }
};
function error(e) {
  ui.note(e.message);
  $("help").textContent = e.message;
}
$("go").onclick = async () => {
  if (busy) {
    if (connecting) {
      connecting.abort();
      $("go").disabled = true;
    }
    return;
  }
  busy = true;
  $("go").disabled = true;
  $("thinking").disabled = true;
  try {
    if (voice) {
      const old = voice;
      voice = null;
      await old.stop();
      return;
    }
    connecting = new AbortController();
    $("go-label").textContent = "Cancel connection";
    $("go").disabled = false;
    const opened = await openVoice(
      token,
      {
        evidence: (e) => {
          window.realtimeEvidence = e;
        },
        status: (t) => {
          $("help").textContent = t;
        },
        analyzers: ui.analyzers,
        transcript: ui.transcript,
        userTranscript: ui.userTranscript,
        removeInput: ui.removeInput,
        permission,
        ended: () => {
          voice = null;
          $("thinking").disabled = false;
          permission(null);
          ui.analyzers(null, null);
          ui.connection(false, $("help").textContent);
        },
        event: (type, d) => {
          if (type === "ready") {
            ui.connection(
              true,
              "Speak naturally. You can interrupt. Headphones recommended.",
            );
            ui.state("LISTEN");
          }
          if (type === "speech_started") {
            ui.state("LISTEN");
            received = 0;
            finished = false;
            measured = false;
            outputItem = null;
          }
          if (type === "speech_stopped") {
            ui.state("SPEC");
          }
          if (type === "input_cleared") {
            ui.state("LISTEN");
          }
          if (type === "audio_received") {
            if (outputItem !== d.itemId) {
              outputItem = d.itemId;
              received = 0;
              finished = false;
            }
            received = Math.max(received, d.startSample + d.samples);
            ui.state("REPLY");
          }
          if (type === "response_done") {
            finished = true;
          }
          if (
            type === "played" &&
            d.itemId === outputItem &&
            !measured &&
            Number.isFinite(d.firstSoundLatencyMs) &&
            d.firstSoundLatencyMs >= 0
          ) {
            $("latency").textContent =
              `FIRST SOUND ${Math.round(d.firstSoundLatencyMs)} ms`;
            measured = true;
          }
          if (
            type === "played" &&
            d.itemId === outputItem &&
            finished &&
            d.playedSamples >= received
          )
            ui.state("LISTEN");
          if (type === "playback_stopped") {
            ui.state("LISTEN");
            ui.note("↳ Playback stopped.");
          }
          if (type === "error") ui.note(d.message);
          if (type === "response_limited")
            ui.note(
              "Reply stopped at its output limit. Keep talking; the session is still live.",
            );
        },
      },
      { thinking: $("thinking").value, signal: connecting.signal },
    );
    voice = opened;
  } catch (e) {
    if (connecting?.signal.aborted) {
      ui.connection(false, "Connection cancelled.");
    } else error(e);
  } finally {
    connecting = null;
    busy = false;
    $("go").disabled = false;
    $("thinking").disabled = !!voice;
  }
};
window.addEventListener("pagehide", () => {
  connecting?.abort();
  voice?.stop(false);
});
