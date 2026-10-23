# Huddle Screen Sharing

Implements the screen-sharing portion of GitHub issue #6384 for desktop huddles.
The relay remains the workspace: no external meeting service, TURN server,
peer-to-peer transport, or media upload is introduced.

## Scope

This implementation continuously captures and encodes H.264 screen video,
targeting 30 FPS at up to 1920 x 1080. It uses inter-frame compression, not JPEG
snapshots. Wayland uses the OS ScreenCast chooser once, then keeps one
PipeWire/GStreamer capture session open until stop. macOS, Windows, and X11 use
`xcap` for the selected source. Camera video and mobile are outside this change.
Complex scenes can exceed the per-packet budget despite average bitrate control.
The encoder performs bounded downscaling for those frames and retains the lower
resolution for the share; capture cadence is unchanged.

The new kind is `24811`, in the Nostr ephemeral range. Frames and stops use the
same `h`-scoped authenticated subscription and membership gates as existing
ephemeral channel events. Frames never enter persistent message history.
Publisher identity comes from the signed event, not a client-supplied sender
field. Access units are timestamped and batched in groups of up to eight; the
native video engine plays those frames at video cadence, not relay event cadence.
Brief, explicitly acknowledged WebSocket quota refusals use bounded cooldowns
without ending the capture. Rejected and pre-cooldown pixels are discarded,
not replayed. A zero-second hint still backs off; persistent refusals, long
quota hints, disconnects, and capture failures remain terminal and visible.

## Manual Validation

Use a native development build and a relay built from this branch. Existing
relays forward ephemeral kinds but do not enforce this kind's H.264/huddle
validation; both sides must be upgraded for the complete contract.
Older relays may also charge ephemeral media against the stored-message minute
quota and report the refusal as a connection NOTICE. Video publication then
stalls waiting for an event acknowledgement. Upgrade relay admission as well as
the desktop; reducing video cadence to fit a chat quota is not a streaming fix.

1. Join the same huddle from two desktop clients.
2. Select Share screen. Verify Share is disabled until a source is selected.
3. Choose a window, then Share. The second client should see that window's
   contents in its huddle room. Scrolling and cursor movement should be smooth,
   with real video playback. Open and close the expanded view.
4. Verify microphone audio continues in both directions.
5. Stop sharing. The remote image should disappear. Immediately share another
   source; an old capture or stop must not replace/remove the new share.
6. Close the companion window while sharing. Sharing should continue from the
   main window. Reopen the companion and verify the image and Stop control.
7. Close the selected source window, deny OS screen permission, and disconnect
   the relay. Verify capture/publication errors are visible and the remote image
   expires within five seconds. Sharing must never restart without user action
after a terminal failure.
An unavailable microphone does not prevent joining a listen-only huddle or
sharing a screen. WebKit input-constraint failures get one minimal retry that
preserves an explicitly selected microphone; permission and device-access
failures remain visible.
8. Leave/end the huddle, switch communities, and rejoin. Capture must be stopped,
   no former frame may appear, and Share must require a new explicit selection.
9. Confirm a non-member and a client in another community cannot receive or
   publish kind `24811` for this huddle.
10. Query kind `24811` after sharing stops. No frames should be returned.

On macOS grant Buzz Screen Recording permission and restart the native app if
the OS requests it. On Wayland, choose a source in the OS picker and verify
there is exactly one permission session. Stop or leave while selection is
pending, and verify that both the picker and any stream are retired.

## Automated Coverage

- `cargo test -p buzz-core huddle_screen`: actual key/delta H.264 frames,
  SPS dimension/profile validation, freshness, batch bounds and channel routing.
- `cargo test -p buzz-relay --lib huddle_screen_tests`: actual ephemeral ingest
  rejects global/ambiguous scope and propagates a channel lookup outage.
- `cargo test -p buzz-test-client --test e2e_huddle_screen -- --ignored`:
  requires an isolated local relay, Postgres, and Redis. Forty production-encoded
  timed frames and a stop are delivered to a member without entering history.
  A non-member and the same community owner in another tenant cannot read or
  publish the stream. Ambiguous scope, forged SPS dimensions, and publication
  after the native client's huddle archive event are refused.
  The sustained-video test publishes 480 batches at capture cadence for over two
  minutes, requiring acknowledgement and delivery of every batch past the
  stored-message quota while leaving history empty.
- Native `huddle::screen::encoder` tests: 60 moving frames are encoded as
  keyframes and deltas, grouped into fewer than 12 valid relay events. Ultrawide
  desktops and window resizes produce matching SPS dimensions and fresh IDRs;
  unusably small sources receive a clear error before reaching OpenH264. A complex
  scene after quiet video adapts within the packet budget, retains pending frames,
  and continues decoding.
- Native `huddle::screen::portal` tests: actual GStreamer video metadata with
  padded rows is respected when producing encoder pixels. The production
  pipeline offers square pixels to PipeWire before scaling, preventing an
  ultrawide capture from negotiating an invalid aspect ratio and collapsing.
- Desktop `screenShare.test.mjs` and `screenPublisher.test.mjs`: stale
  sequences, stop/replacement fences, viewer expiry, bounded recipients,
  single-flight publication, expiry while another sender stays live, and
  stopping during an in-flight capture. Production publisher tests also cover
  zero-second quota hints, fresh-frame recovery, cancellation during cooldown,
  and bounded terminal retries.
- Scoped access-revocation tests: a huddle-scoped listener does not receive
  another channel's revocation hint.
- `huddleMicrophone.test.mjs` and `huddle-microphone.spec.ts`: unsupported
  constraints, unavailable input, selected-device fidelity, permission refusal,
  acquisition cancellation, and an active mic-less huddle with screen controls.
- `pnpm test:e2e:smoke huddle-screen-sharing.spec.ts`: real UI chooser,
  native state mirroring, actual H.264 decoding and moving pixels, presented
  frame-rate checks, expansion, explicit decoder retry, terminal stop, and expiry.
  A 75-second playback test exercises repeated media-buffer cleanup.
  The native IPC boundary and relay are mocked; these tests cannot establish
  OS permission/capture behavior or live relay isolation.

Native and live-relay validation plus explicit human confirmation are required
before marking a PR ready. Browser screenshots are not a substitute.
