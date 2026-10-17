# Huddle Camera Video

Desktop participants can explicitly enable a webcam in a huddle. Camera video
replaces the participant's avatar in room tiles, with a mirrored self-preview.
Receiving video does not require a camera. Audio and screen sharing remain
independent; camera detection never opens a device or requests microphone access.

This extends the screen-sharing transport in PR #8321 and the huddle video
request in [issue #6384](https://github.com/block/buzz/issues/6384). It does not
implement mobile camera publishing or viewing.

## Architecture

- Camera video is signed, ephemeral Nostr kind `24812`; screen video remains
  kind `24811`. Both use exact `h` channel scoping and existing community,
  membership, and active-private-huddle admission. Frames are not stored.
- Browser `getUserMedia` acquires video only, after an explicit action. Device
  selection is origin-local and never published. Constraint errors receive one
  minimal retry that preserves an explicitly selected camera; permission,
  unavailable-device, and busy-device failures propagate.
- Discovery runs on huddle entry, picker opening, explicit refresh, and after
  capture permission succeeds. Overlapping refreshes share one probe. There is
  no app-idle enumeration or `devicechange` listener, avoiding the re-announcement
  feedback loop documented in [PR #7866](https://github.com/block/buzz/pull/7866).
  Discovery failures retain the previous list and expose a retryable error.
- The main webview owns the MediaStream, frame pump, and publisher. Companion
  controls delegate intent and mirror native state. A late permission grant
  cannot resurrect a cancelled consent identity.
- One bounded RGBA frame is submitted at a time over binary Tauri IPC. Native
  OpenH264 targets 30 FPS, up to 1280 x 720, at 1.2 Mbps. Periodic keyframes,
  adaptive downscaling, and a two-batch queue bound work and memory.
- Camera and screen share the H.264 wire validator, signed packet reader,
  receiver, and player. Kind and consent identity keep their queues and
  lifetimes separate. Reads are fenced before and after signing.
- Session-owned worker clocks avoid background window-timer throttling.
  Stop, source failure, leave/end, signer changes, and community changes retire
  browser tracks, workers, queued frames, and native reservations.
- Viewers expire disconnected streams and release decoder resources. Explicit
  stop is scoped to its consent identity; an old stop cannot remove a replacement.

The feature keeps the existing relay-only architecture: no external SFU,
peer-to-peer mesh, image uploads, or additional HTTP API.

## Platform Compatibility

Upgrade both the sending and receiving desktop clients. A screen-sharing-only
client cannot subscribe to or display camera video. Upgrade the relay for the
camera-specific validation and huddle admission guards; an older relay may
forward the ephemeral kind without them.

Camera usage descriptions and macOS camera entitlements cover the opt-in flow.
Browser detection retains redacted device entries and offers system-default
capture, rather than treating pre-permission labels as missing hardware.
The main window alone enumerates cameras. Companion pickers request its device
list over channel-scoped local app events, because browser camera permission and
device IDs belong to the capture-owning webview. Requests are correlated and
bounded by a timeout; stale replies cannot repopulate a retired huddle. Discovery
does not open a camera, and device IDs never leave the app.
Relevant reports include [Tauri #6623](https://github.com/tauri-apps/tauri/issues/6623)
(device release), [#15905](https://github.com/tauri-apps/tauri/issues/15905)
(redacted device enumeration), and [#15277](https://github.com/tauri-apps/tauri/issues/15277)
(Linux permission integration). Track cleanup is tested, but an OS camera LED
or permission dialog still requires platform verification.

**Known Linux dependency issue:** WebKitGTK 2.52.6 with GStreamer 1.28 can freeze
`getUserMedia` on its first frame. WebKit's zero-base-time capture pipeline lets
`videorate` fill the machine's uptime with duplicates. See
[WebKit bug 324582](https://bugs.webkit.org/show_bug.cgi?id=324582) and
[the proposed fix](https://github.com/WebKit/WebKit/pull/74373), which sets
`skip-to-first=true`. Physical Linux verification on this affected combination
used an app-local compatibility workaround. This PR does not bundle that
workaround or claim unpatched affected runtimes work.

## Validation

- Full `just ci`, isolated `just test`, and relay-backed video tests passed.
  Relay coverage includes independent camera/screen tracks, membership and
  tenant isolation, no persisted frames, and sustained screen-video delivery.
- Unit and browser regressions exercise opt-in, video-only constraints, exact
  device selection, cancellation, raw IPC geometry, background-clock scheduling,
  decoding, demand-driven discovery, re-announcement storms, and stop cleanup.
  Browser tests mock native
  IPC and relay transport; they do not establish OS permissions.
- Native OpenH264 tests encode, validate, and decode moving camera frames with
  inter-frame compression and periodic keyframes.
- Linux native synthetic video exercised binary IPC, native encoding, a local
  relay, and a second native viewer. Physical Linux capture was also exercised,
  including a hidden owner, with the dependency workaround noted above.
- On Apple Silicon macOS 26.4 without a webcam, a 0.5.28 release build and
  deep/strict ad-hoc signature verification passed. Native huddle and focused
  JavaScript tests passed. A separate native Tauri/WKWebView harness exercised
  the production camera pipeline with a generated 720p source and controlled
  local transport: a minute-long run averaged 25.5 presented FPS, followed by
  stop/restart, retired-session fences, and simulated-disconnection cleanup.
  FPS used presented-frame callbacks because decoder statistics could be stale.
- The human confirmed live Linux-camera video displayed on the Mac after both
  clients were upgraded. On October 16, 2026, the human also tested the final
  owner-mediated companion picker in the installed Linux build and confirmed
  selection of the monitor webcam works.

Mac camera hardware/permissions, physical LED release, Windows capture, and
minimized-window behavior still need platform checks. Generated canvas streams
do not establish physical-camera behavior while minimized.

## Manual Validation

1. Join without a webcam. Receiving video, audio, screen sharing, and leaving
   remain available; the camera control reports no camera.
2. Connect a webcam and open Camera settings (or choose Refresh cameras) to
   refresh detection. It must not enable itself. Explicitly enable it and
   grant permission; another participant
   should see moving video in your tile, not images in message history.
3. Select a second camera. The old track must stop before the replacement opens.
   An unavailable selected camera must not silently fall back to another.
4. Share the camera and screen together, then stop each separately. The other
   track and audio should continue. Verify the supported minimum window size.
5. Disable the camera and check its physical LED/OS indicator. Remote video must
   disappear. Re-enable explicitly, including from the companion window.
6. Deny permission, cancel a pending grant, unplug the camera, disconnect the
   relay, leave/end, and switch community/identity. No stale frame may revive an
   old consent identity or enter another huddle; failures must permit retry.
7. Close the companion and minimize the owner separately. Verify sustained
   playback on a second client, then confirm stop releases the actual device.
8. Query kind `24812` after sharing ends. No camera frames should be returned.
