# Linux Rendering Troubleshooting

This guide covers the most common rendering failures on Linux and how to resolve them. It covers both the AppImage distribution and native package installs (`deb`, `rpm`).

## Symptoms and fixes at a glance

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Blank or transparent window, then `SIGABRT` with `colrv1_configure_skpaint` in the output | COLRv1 color emoji font (AppImage only) | Upgrade to the latest AppImage (v0.5.2+) |
| Blank window on startup / SIGSEGV when switching workspaces | dmabuf renderer incompatibility (NVIDIA or AppImage) | Prefer `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` (shipped automatically) or `--safe-rendering`. Do **not** set `WEBKIT_DISABLE_DMABUF_RENDERER=1` on current WebKitGTK — see [#3654](https://github.com/block/buzz/issues/3654). On Debian/Ubuntu with the proprietary NVIDIA driver the crash can persist (distro WebKit patch) — [#3654](https://github.com/block/buzz/issues/3654) stays open for that path. |
| Blank window on any hardware, no crash output | Unknown GPU/driver combination | `--safe-rendering` flag (see below) |
| UI paints, then the window freezes and the web process `SIGABRT`s ~20-30s in | A camera whose caps carry no `framerate` field (Intel IPU6 laptops expose one via libcamera) | Hide the offending camera — see [Crash after the UI loads](#crash-after-the-ui-loads-camera-enumeration) |

---

## Crash: `colrv1_configure_skpaint` assertion abort (AppImage)

**Affected distributions:** Fedora 40+ and any distro shipping Google's Noto Color Emoji in COLRv1 format (`Noto-COLRv1.ttf`). Issues [#2548](https://github.com/block/buzz/issues/2548), [#2982](https://github.com/block/buzz/issues/2982).

**Symptom:** Buzz starts, the window appears briefly (or stays blank), then the process aborts with output like:

```
././/include/c++/12/bits/stl_vector.h:1123: ... colrv1_configure_skpaint ...:
Assertion '__n < this->size()' failed.
```

**Root cause:** The AppImage bundles WebKitGTK compiled against FreeType 2.11.1 (Ubuntu 22.04's version), but `libfreetype.so.6` is not bundled — WebKit loads the host's FreeType at runtime instead. FreeType 2.13.0 (2023-02-09) added a field to `FT_ColorStopIterator`, growing the struct from 16 to 20 bytes. On Fedora 40+ hosts (FreeType ≥ 2.13), the struct-layout mismatch corrupts color-stop index arithmetic inside Skia's COLRv1 renderer, producing the assertion abort.

**Fix:** Upgrade to the latest AppImage (v0.5.2+). The build container was bumped to `ubuntu:24.04` ([#3602](https://github.com/block/buzz/pull/3602)), which ships FreeType 2.13.2. The compiled layout now matches every crash-affected host (FreeType ≥ 2.13), eliminating the ABI mismatch.

**AppImage glibc floor (v0.5.2+):** The `ubuntu:24.04` build raises the AppImage's minimum glibc requirement:

| AppImage version | glibc floor | Oldest supported AppImage distro |
|---|---|---|
| v0.5.1 and earlier | 2.35 | Ubuntu 22.04 LTS, Debian 12 |
| v0.5.2+ | 2.39 | Ubuntu 24.04 LTS, Fedora 40+ |

If you are on **Ubuntu 22.04 LTS or Debian 12**, upgrade to the latest **`.deb`/`.rpm`** package instead — native packages use the system WebKit and are unaffected by this change.

**Workaround (before upgrading):** Add a fontconfig override that removes color-format fonts from Buzz's view:

```bash
mkdir -p ~/.config/buzz-fontconfig
cat > ~/.config/buzz-fontconfig/fonts.conf <<'XML'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
  <selectfont>
    <rejectfont>
      <pattern>
        <patelt name="color"><bool>true</bool></patelt>
      </pattern>
    </rejectfont>
  </selectfont>
</fontconfig>
XML
FONTCONFIG_FILE=~/.config/buzz-fontconfig/fonts.conf ./Buzz_*.AppImage
```

**Native packages (`deb`/`rpm`):** The COLRv1 crash ([#2548](https://github.com/block/buzz/issues/2548), [#2982](https://github.com/block/buzz/issues/2982)) is AppImage-only — native packages use the system WebKit, which has a consistent FreeType ABI, and are not affected.

---

## Blank window on startup (no crash): dmabuf renderer

**Affected hardware:** NVIDIA GPUs (proprietary and nouveau drivers) and AppImage installs on any GPU. Issue [#2338](https://github.com/block/buzz/issues/2338).

**Symptom:** Buzz launches without any crash or assertion output, but the window is blank or invisible. The process is running (`ps aux | grep buzz`), but nothing renders.

**Root cause:** WebKitGTK's dmabuf zero-copy buffer path is incompatible with some GPU/driver/compositor combinations. The WebKit child process silently fails to paint.

**Fix (shipped automatically starting with the first release containing [#3271](https://github.com/block/buzz/pull/3271) (v0.5.1), updated for [#3654](https://github.com/block/buzz/issues/3654)):** Buzz sets `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` automatically before WebKit initializes when it detects an NVIDIA GPU (`/sys/class/drm` vendor ID `0x10de`) or when running as an AppImage. That keeps SharedMemory in WebKit's transport set while skipping the hardware dmabuf path (upstream WebKitGTK; Debian/Ubuntu's NVIDIA dmabuf patch can bypass this, so the crash can persist there — [#3654](https://github.com/block/buzz/issues/3654) stays open for that path). `WEBKIT_DMABUF_RENDERER_FORCE_SHM` exists in WebKitGTK ≥ 2.44 (absent at 2.42); on older system WebKit the export is a silent no-op.

**Do not use `WEBKIT_DISABLE_DMABUF_RENDERER=1` on current WebKitGTK** (2.52+): that variable no longer falls back to shared memory. It empties the transport mode, `AcceleratedBackingStore::create()` returns null, and the UI SIGSEGVs the first time compositing is needed (often on workspace switch). See [#3654](https://github.com/block/buzz/issues/3654).

**If automatic detection doesn't help (`--safe-rendering`):** Pass `--safe-rendering` to force both `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` and `WEBKIT_DISABLE_COMPOSITING_MODE=1` for that launch:

```bash
./Buzz_*.AppImage --safe-rendering
# or for a native install:
buzz-desktop --safe-rendering
```

`--safe-rendering` is a per-launch flag — it is not remembered between runs. If it fixes your issue, you can make it permanent by setting the env vars yourself:

```bash
# ~/.bashrc or ~/.profile
export WEBKIT_DMABUF_RENDERER_FORCE_SHM=1
```

**Conflict detection:** If you set a WebKit variable in your environment and also pass `--safe-rendering`, Buzz will refuse to start and print exactly which variable conflicts. Unset the conflicting variable or drop the flag. Operators who previously exported `WEBKIT_DISABLE_DMABUF_RENDERER=0` to override the old heuristic can keep that — the module still treats that assignment as a user takeover.

---

## AMD RDNA4 / transparent window

**Affected hardware:** AMD RDNA4 GPUs (RX 9000 series) with the `radv` driver. Issue [#2643](https://github.com/block/buzz/issues/2643).

**Symptom:** The Buzz window is transparent or renders with graphical corruption on AMD RDNA4 hardware.

**Workaround (recommended; FORCE_SHM swap not re-verified on RDNA4):** Set these three variables before launching Buzz. The reporter originally verified a three-var set that used `WEBKIT_DISABLE_DMABUF_RENDERER=1`; that var is the [#3654](https://github.com/block/buzz/issues/3654) crash on current WebKitGTK, so this recipe swaps in `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` instead. Please re-confirm on RDNA4 if you can.

```bash
export GDK_BACKEND=x11
export WEBKIT_DMABUF_RENDERER_FORCE_SHM=1
export WEBKIT_SKIA_ENABLE_CPU_RENDERING=1
./Buzz_*.AppImage
# or for native:
buzz-desktop
```

- `WEBKIT_SKIA_ENABLE_CPU_RENDERING=1` forces Skia to use CPU rendering, bypassing the RDNA4 Skia/radv paint failure.
- `GDK_BACKEND=x11` avoids the blank window that appears when running under a Plasma-Wayland compositor.
- `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` keeps shared-memory transport without the #3654 empty-mode crash from `WEBKIT_DISABLE_DMABUF_RENDERER` (needs WebKitGTK ≥ 2.44).

A dedicated fix for RDNA4 detection is being tracked in [#2643](https://github.com/block/buzz/issues/2643).

---

## Crash after the UI loads: camera enumeration

**Affected hardware:** Laptops whose camera stack publishes a capture device with
no `framerate` field in its caps. Confirmed on Intel IPU6 machines (Dell XPS 9320,
sensor `ov01a10`) on Ubuntu 26.04 with webkit2gtk 2.52.3. Issue
[#6339](https://github.com/block/buzz/issues/6339).

**Symptom:** Buzz starts normally and the UI paints — you see the loading screen or
the app itself. Roughly 20-30 seconds later the window freezes on whatever it last
drew and never progresses. There is no error in Buzz's output. The web process is
gone:

```bash
pgrep -x WebKitWebProces || echo "web process died"
```

This is easy to misread as a hang or a slow network. It is a crash, and because it
lands *after* first paint, any check taken in the first ~15 seconds reports a
healthy process.

**Root cause:** This is a WebKitGTK bug, not a Buzz bug. When a page calls
`navigator.mediaDevices.enumerateDevices()`, WebKit builds video presets for every
capture device it can see. `GStreamerVideoCaptureSource.cpp` handles a `framerate`
that is a fraction range or a single fraction, and otherwise assumes a list. When
the field is absent entirely, `gst_structure_get_value()` returns `NULL` and
`gstStructureGetList()` aborts the whole web process on a `RELEASE_ASSERT` that sits
one line above its own graceful early-return. Symbolicated:

```
#2  abort ()
#3  WTFCrashWithInfo(int, char const*, char const*, int)   WTF/Assertions.h:987
#4  gstStructureGetList<double>()                          GStreamerCommon.cpp:1460
#5  generatePresets()                                      GStreamerVideoCaptureSource.cpp:326
#8  capabilities()                                         GStreamerVideoCaptureSource.cpp:256
#25 enumerateDevices()                                     RealtimeMediaSourceCenter.cpp:307
```

On IPU6 laptops the sensor reaches userspace twice. The usable device, fed by
`icamerasrc` through `v4l2loopback`, is fine. The libcamera duplicate is not:

```
Intel MIPI Camera (V4L2)      video/x-raw, format=NV12, width=1280, height=720, framerate=30/1
Built-in Front Camera         video/x-raw, format=RGBA, width=640, height=480, colorimetry=sRGB
```

**Check whether you are affected:**

```bash
gst-device-monitor-1.0 Video/Source 2>/dev/null | grep 'caps  :' | grep -v framerate
```

Any line printed is a device that will abort WebKit. No output means this is not
your problem.

**Workaround:** Hide the framerate-less duplicate. Both steps are needed — PipeWire
and GStreamer enumerate libcamera independently — and neither disturbs the working
camera, which does not go through libcamera:

```bash
# 1. PipeWire's libcamera backend
mkdir -p ~/.config/wireplumber/wireplumber.conf.d
cat > ~/.config/wireplumber/wireplumber.conf.d/50-disable-libcamera.conf <<'CONF'
wireplumber.profiles = {
  main = {
    monitor.libcamera = disabled
  }
}
CONF
systemctl --user restart wireplumber

# 2. GStreamer's own libcamera device provider
sudo apt remove gstreamer1.0-libcamera
```

Re-run the check above; it should print nothing. Note that removing
`gstreamer1.0-libcamera` also removes GNOME's Camera app (`gnome-snapshot`), which
depends on it.

Verify your camera still works afterwards — on IPU6 the capture path is
`icamerasrc` → `v4l2loopback`, so it is unaffected:

```bash
gst-device-monitor-1.0 Video/Source 2>/dev/null | grep 'name  :'
```

**Upstream:** The fix belongs in WebKit — the `RELEASE_ASSERT` should be dropped so
the existing `GST_WARNING` and early return can skip the device instead of killing
the process. Still present on WebKit `main` at the time of writing
(`GStreamerCommon.cpp:1403-1408`).

---

## Diagnosing an unrecognised crash

If none of the above match your situation:

1. Run Buzz from a terminal and capture the output:
   ```bash
   ./Buzz_*.AppImage 2>&1 | tee buzz-crash.log
   ```

2. Check for a core dump:
   ```bash
   coredumpctl list | tail
   coredumpctl info <PID>
   ```

3. Try `--safe-rendering` first — if it resolves the issue, it's a WebKit rendering incompatibility and the crash log will help narrow down which driver is involved.

4. File a [new issue](https://github.com/block/buzz/issues/new) with your distro, GPU, driver version, and the terminal output.
