# Linux: libcamera device-enumeration feedback loop (file-descriptor exhaustion)

This guide covers a Linux-only failure where the app spams libcamera camera logs in a
tight loop and eventually exhausts its file descriptors — without the user ever using
the camera. It covers both the AppImage distribution and native installs.

**Status:** diagnosis confirmed on v0.5.24, including a falsification
experiment (see "Falsification result" below). The app-side fix is implemented
**and hardware-validated** on the affected machine (2026-09-23; see "Hardware
validation" below); the diagnostic procedure in this document doubles as a
stopgap.

## Symptoms and fixes at a glance

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Endless `INFO Camera camera_manager.cpp:340 libcamera` + `Adding camera ... for pipeline handler uvcvideo` lines on startup, rate accelerating over time, then "too many open files" / app degradation | `enumerateDevices()` ↔ `devicechange` feedback loop through WebKitGTK's GStreamer device providers (needs a distro libcamera GStreamer plugin + a camera present) | Remove/blacklist the host libcamera GStreamer plugin (stopgap, see below); app-side fix removes the feedback loop |
| Same log spam but no FD growth | Harmless noise from a one-shot enumeration | No action needed |

## Symptom

On startup the terminal fills with repeating libcamera lines at ~0.3–0.7 s cadence
(bracketed numbers are logging-thread TIDs and climb steadily):

```
[2:22:00.503205232] [99346]  INFO Camera camera_manager.cpp:340 libcamera v0.7.0
[2:22:00.602256256] [99450]  INFO Camera camera_manager.cpp:223 Adding camera '\_SB_.PCI0.XHCI.RHUB.HS05-5:1.0-0c45:636b' for pipeline handler uvcvideo
[2:22:00.613337813] [99449]  INFO Camera camera_manager.cpp:340 libcamera v0.7.0
...
```

Each `camera_manager.cpp:340` line is a **brand-new `libcamera::CameraManager`
instance being constructed**; each `Adding camera ...` line is that fresh manager
re-detecting the webcam. Over time the cadence bunches into bursts (2 managers per
cycle growing to 4+), the TID climb accelerates, and the process eventually runs out
of file descriptors (EMFILE). The user is not using the camera.

## Root cause

A feedback loop with three necessary parties. The app's event-driven re-enumeration
is the amplifier, WebKitGTK's monitor-per-enumeration behavior is the engine, and
libcamera is the loud, heavy amplifier that made the loop visible:

1. **Buzz (amplifier).** `HuddleProvider` wraps the whole app
   (`desktop/src/app/AppHuddleShell.tsx`) and mounts at startup.
   `useAudioDevices` (`desktop/src/features/huddle/lib/useAudioDevices.ts`) calls
   `navigator.mediaDevices.enumerateDevices()` on mount **and re-enumerated on every
   `devicechange` event** — an unbounded, event-driven retry loop with no backoff and
   no terminal state. `AnimatedAvatarCapture` ran the same pattern while its modal
   was open. (The output-device listener in `HuddleContext` was loop-safe — it
   queries the Rust cpal backend, not `mediaDevices`.)
2. **WebKitGTK (engine).** Media capture is enabled on Linux
   (`desktop/src-tauri/src/linux_media.rs`, `set_enable_media_stream(true)`), so
   `mediaDevices` exists. **Every `enumerateDevices()` call makes WebKitGTK's
   `GStreamerCaptureDeviceManager` start a fresh `GstDeviceMonitor`**, which starts
   **all** matching device providers — audio (pulse) *and* video. Video monitors
   include the distro's libcamera provider, which constructs a fresh
   `CameraManager` per start — hence the log lines. WebKitGTK fires `devicechange`
   on DEVICE_ADDED
   messages and has a flush trick for messages queued during the initial provider
   probe (`gst_bus_set_flushing` in upstream
   `Source/WebCore/platform/mediastream/gstreamer/GStreamerCaptureDeviceManager.cpp`),
   but providers that add devices asynchronously *after* that flush window deliver
   them as `devicechange`. The measured result (below) shows this re-announcement is
   not specific to libcamera: **each monitor start re-announces already-known
   devices**, so any `devicechange`-triggered re-enumeration re-announces again,
   forever.
3. **libcamera + gst plugin (loud, heavy amplifier).** Each fresh `CameraManager` spawns
   threads and opens device nodes (video nodes, media-controller devices, eventfds).
   When the loop runs, these accumulate faster than they are released and the process
   exhausts its FDs.

### Falsification result (v0.5.24, Linux AppImage)

Hiding the libcamera plugin (procedure below) produced outcome 2 of the
interpretation matrix: the libcamera log spam disappeared entirely, but the
WebKitWebProcess FD count still exploded (117 → 634 within minutes) and the web
process died, freezing the app. The main process stayed flat (~53–60 FDs).

This proves the loop does **not** need libcamera. With only the v4l2 and pulse
providers active, WebKitGTK still re-announces devices on every monitor start,
feeding the app's `devicechange` listeners, and each enumeration cycle leaks file
descriptors in the web process by itself. libcamera was only the loudest, heaviest
amplifier — the engine is WebKitGTK's monitor-per-enumeration plus the app's
event-driven re-enumeration. The app fix below therefore removes the event-driven
re-enumeration instead of relying on hiding system plugins, and a WebKitGTK upstream
bug (draft at the bottom) covers the per-enumeration re-announcement and FD leak.

Why it triggers without using the camera: the trigger is the huddle **audio**-device
listing; a single `enumerateDevices()` scans audio *and* video providers, so the
webcam's libcamera provider gets spun too. Nothing ever captures from it.

Why not every Linux user hits it: it requires a distro that ships the GStreamer
libcamera plugin **and** a camera the libcamera uvcvideo pipeline handler matches.
Check with `gst-inspect-1.0 libcamera` (shows `libcameraprovider: libcamera Device
Provider` when present).

## Proving the diagnosis (and stopgap): hide the libcamera GStreamer plugin

`gst-inspect-1.0 -b` does **not** work for this — `-b`/`--print-blacklist` only
*prints* entries already recorded in the binary registry cache (populated when a
plugin fails to load). GStreamer 1.x has no CLI to add blacklist entries, and the
registry cache is a binary format not meant for hand editing. The reliable test is to
make the plugin file temporarily invisible to GStreamer:

```bash
# 1. Find the plugin file (from your gst-inspect output):
gst-inspect-1.0 libcamera
#    Filename: /usr/lib/x86_64-linux-gnu/gstreamer-1.0/libgstlibcamera.so

# 2. Hide it (package-manager-owned file — move, do not delete):
sudo mv /usr/lib/x86_64-linux-gnu/gstreamer-1.0/libgstlibcamera.so \
        /usr/lib/x86_64-linux-gnu/gstreamer-1.0/libgstlibcamera.so.disabled

# 3. Force a clean registry rebuild for your user:
rm -f ~/.cache/gstreamer-1.0/registry.x86_64.bin

# 4. Confirm it is gone:
gst-inspect-1.0 libcamera        # → "No such element or plugin 'libcamera'"

# 5. Run Buzz and watch FD counts:
./Buzz_0.5.24_amd64.AppImage
for p in $(pgrep -f 'buzz-desktop|WebKitWebProcess'); do
  echo "$p: $(ls /proc/$p/fd 2>/dev/null | wc -l) fds";
done   # repeat a few times over a couple of minutes
```

Interpreting the result:

- **Spam gone, FD counts flat, everything works** → libcamera provider involvement
  confirmed. The app-side fix below is still needed: it removes the amplifier so no
  system file changes are required.
- **Spam gone but FD counts still climb** → the enumerate/monitor cycle itself leaks
  independent of libcamera; the app-side fix is the actual cure, and a WebKitGTK
  upstream bug should be filed. *(Measured on v0.5.24: spam gone, WebKitWebProcess
  FDs 117 → 634 within minutes, then the process died and the app froze; main
  process flat at ~53–60. See "Falsification result".)*
- **Spam continues** → the model is wrong; capture the new log and reopen the
  diagnosis.

Restore afterwards:

```bash
sudo mv /usr/lib/x86_64-linux-gnu/gstreamer-1.0/libgstlibcamera.so.disabled \
        /usr/lib/x86_64-linux-gnu/gstreamer-1.0/libgstlibcamera.so
rm -f ~/.cache/gstreamer-1.0/registry.x86_64.bin
```

UVC webcams remain usable without the libcamera plugin (the v4l2 device provider in
gst-plugins-good enumerates them), and pulse audio is unaffected. Note the AppImage
uses the **host's** GStreamer plugins — `desktop/scripts/fix-appimage.sh` strips
bundled ones and unsets the bundle-pointing `GST_PLUGIN_SYSTEM_PATH` overrides — so
this plugin always comes from the distro.

## Fix (implemented in the desktop app)

Goal: enumeration is demand-driven and bounded, so the app can no longer amplify
provider noise into a loop. All changes are desktop frontend. The earlier plan's
"re-enumerate on a slow interval while a huddle is active" idea was dropped: the
falsification result shows *every* enumeration cycle leaks FDs in the web process,
so any recurring trigger — even time-driven — multiplies the leak. The device list
is refreshed only at the moments its contents can have changed or are about to be
shown.

1. **`desktop/src/features/huddle/lib/useAudioDevices.ts`**
   - No mount-time enumeration and no `devicechange` listener (the loop driver is
     gone); the WebKitGTK re-announcement hazard is documented in the hook.
   - A single serialized `refreshAudioDevices()`: an in-flight guard makes
     overlapping calls await the running enumeration (never overlap), and a
     content-equality skip means an unchanged `audioinput` list does not update
     React state.
   - Exposed through `useHuddle()` (a no-op in companion windows, which mirror the
     audio-owning window's list).
2. **Triggers.** The mic picker opening (`MicControls` `onPickerOpen`, wired via
   the picker popover's `onOpenChange`) and exactly one refresh after
   `getUserMedia` succeeds in a huddle start/join (`connectAndSetupMedia` —
   device labels only become readable once capture permission is granted).
3. **Output devices (`HuddleContext.tsx`).** The `devicechange` handler only
   queries the Rust `list_audio_output_devices` command (cpal/rodio, no
   `mediaDevices` — loop-safe), now behind a ~1 s trailing debounce with an
   identical-list skip so residual event storms cannot spam the backend.
4. **Animated avatar capture (`AnimatedAvatarCapture.tsx`).** `devicechange`
   listener removed; enumerates once when the capture UI opens and on camera
   source switches (user-driven, bounded).
5. **Regression test** — `desktop/src/features/huddle/lib/useAudioDevices.test.mjs`,
   using the established `node:test` + jsdom + `act` pattern from
   `useVoiceNoteRecorder.test.mjs`. The fake `mediaDevices` reproduces the
   pathological WebKitGTK behavior: **every** `enumerateDevices()` schedules a
   `devicechange` after resolving. Asserts: zero enumerations at mount, zero
   additional enumerations from 50 simulated re-announcements, overlapping refreshes
   serialize into one enumeration, and unchanged device lists do not update state.
   The test fails if event-driven enumeration is reintroduced.
6. **Validation.** `cd desktop && pnpm test && pnpm typecheck`; `just ci` before
   the PR. On the affected machine, restore the libcamera plugin if it was
   hidden (`sudo mv …/libgstlibcamera.so.disabled …/libgstlibcamera.so` and
   `rm -f ~/.cache/gstreamer-1.0/registry.x86_64.bin`), rebuild the AppImage,
   then run the matrix in "Hardware validation" below (done 2026-09-23 —
   result recorded there). The interactive checks (mic picker lists devices,
   huddle join works, animated-avatar capture still enumerates when opened)
   can be done in the same session.

## Hardware validation (2026-09-23, affected machine)

Validated with a 2×2 matrix — artifact (pre-fix stale drop-in vs fixed build)
× launch mode (direct `./Buzz_0.5.24_amd64.AppImage` vs `firejail --novideo
--noprofile`, which is what the local `buzz.sh` wrapper uses). Each run sampled
the WebKitWebProcess fd count every 5 s for 75–150 s and counted
`camera_manager.cpp:223` lines in captured stderr.

| Artifact | Launch | `Adding camera` lines | WebKitWebProcess fds |
|---|---|---|---|
| pre-fix (stale drop-in) | direct | 1687 in 75 s | 91 → 915 in 60 s; leaked fds are `(deleted)` entries; plateau as enumeration fails near the fd limit |
| pre-fix (stale drop-in) | firejail | 0 | flat 71 |
| fixed build | firejail | 0 | flat 71 |
| fixed build | direct | 6, all inside a 240 ms startup burst, then silence | flat 71 for 150 s |

Two methodology traps discovered during validation (together they produced one
false "fix failed" round):

1. **Stale-artifact mixup.** The wrapper picks `Buzz_*.AppImage` from a drop-in
   directory; a same-version pre-fix build left there means "rebuild and
   re-test" silently exercises the old code. Verify artifact identity before
   validating: `sha256sum` of the `--appimage-extract`ed
   `usr/bin/buzz-desktop.bin` compared against the freshly built binary.
2. **firejail `--novideo` masks the bug.** It hides `/dev/video*`, so libcamera
   never matches a camera: even the buggy build logs nothing and stays flat.
   Validation of this bug must run the AppImage **directly** (no firejail
   wrapper), or the test is vacuous.

The surviving `devicechange` listener in `HuddleContext` (output devices, cpal
only) was present in the fixed build during validation and produced no
enumeration churn — listener *registration* alone does not instantiate the
WebKitGTK capture-device machinery's loop; actual `enumerateDevices()` calls
do. A plan to remove that listener was dropped as unnecessary.

Post-fix behavior on direct launch: the bounded startup burst (6
`CameraManager` constructions in 240 ms) comes from WebKitGTK's own
capture-device-manager init, not app code, and does not grow. The remaining
per-enumeration fd leak is the upstream WebKitGTK bug (below).

Remaining known limitation: if WebKitGTK leaks a few FDs per enumeration cycle even
outside the runaway loop, heavy use of the picker leaks slowly. That part is
upstream — file the bug below.

## Upstream WebKitGTK bug

Filed upstream as [Bug 325151](https://bugs.webkit.org/show_bug.cgi?id=325151)
on 2026-09-24; the text below is what was submitted (attachment:
`webkitgtk-repro.html`, a re-enumerating page that logs each trigger).

**Title:** `enumerateDevices()` re-announces known devices as `devicechange` and
leaks file descriptors in the WebKitWebProcess

**Environment:** Linux, WebKitGTK (GStreamer media backend), webcam + pulse audio
devices present; reproducible with the libcamera GStreamer plugin removed.

**Summary:** Each call to `navigator.mediaDevices.enumerateDevices()` starts a
fresh `GstDeviceMonitor`. When that monitor starts, devices already known to the
`GStreamerCaptureDeviceManager` are re-announced as `devicechange` events — the
initial-probe flush (`gst_bus_set_flushing`) does not cover providers that add
devices asynchronously after the probe window. Additionally, repeated enumeration
cycles leak file descriptors in the WebKitWebProcess: measured growth from 117 to
634 open FDs within minutes of event-driven re-enumeration, ending in EMFILE and a
dead web process.

**Impact:** any page that re-enumerates in response to `devicechange` (as the
media-capture spec suggests for keeping device lists fresh) enters an unbounded
enumerate ↔ `devicechange` loop and leaks FDs until the web process dies. A page
cannot distinguish "device actually changed" from "monitor restarted".

**Repro:** a page that calls `enumerateDevices()` on every `devicechange` event
(or on an interval), with at least one camera and one audio device present; watch
`ls /proc/<WebKitWebProcess pid>/fd | wc -l` climb.
