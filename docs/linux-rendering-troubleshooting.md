# Linux Rendering Troubleshooting

This guide covers the most common rendering failures on Linux and how to resolve them. It covers both the AppImage distribution and native package installs (`deb`, `rpm`).

## Symptoms and fixes at a glance

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Blank or transparent window, then `SIGABRT` with `colrv1_configure_skpaint` in the output | COLRv1 color emoji font (AppImage only) | Upgrade to the latest AppImage (v0.5.2+) |
| Blank window on startup, no crash output | dmabuf renderer incompatibility (NVIDIA or AppImage) | `WEBKIT_DISABLE_DMABUF_RENDERER=1 ./Buzz.AppImage` or `--safe-rendering` |
| Blank window on any hardware, no crash output | Unknown GPU/driver combination | `--safe-rendering` flag (see below) |

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

**Fix (shipped automatically starting with the first release containing [#3271](https://github.com/block/buzz/pull/3271) (v0.5.1)):** Buzz sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` automatically before WebKit initializes when it detects an NVIDIA GPU (`/sys/class/drm` vendor ID `0x10de`) or when running as an AppImage. This restores a slightly slower shared-memory rendering path that works universally.

**If automatic detection doesn't help (`--safe-rendering`):** Pass `--safe-rendering` to force both `WEBKIT_DISABLE_DMABUF_RENDERER=1` and `WEBKIT_DISABLE_COMPOSITING_MODE=1` for that launch:

```bash
./Buzz_*.AppImage --safe-rendering
# or for a native install:
buzz-desktop --safe-rendering
```

`--safe-rendering` is a per-launch flag — it is not remembered between runs. If it fixes your issue, you can make it permanent by setting the env vars yourself:

```bash
# ~/.bashrc or ~/.profile
export WEBKIT_DISABLE_DMABUF_RENDERER=1
```

**Conflict detection:** If you set a WebKit variable in your environment and also pass `--safe-rendering`, Buzz will refuse to start and print exactly which variable conflicts. Unset the conflicting variable or drop the flag.

---

## AMD RDNA4 / transparent window

**Affected hardware:** AMD RDNA4 GPUs (RX 9000 series) with the `radv` driver. Issue [#2643](https://github.com/block/buzz/issues/2643).

**Symptom:** The Buzz window is transparent or renders with graphical corruption on AMD RDNA4 hardware.

**Workaround (verified by reporter):** Set these three variables before launching Buzz:

```bash
export GDK_BACKEND=x11
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export WEBKIT_SKIA_ENABLE_CPU_RENDERING=1
./Buzz_*.AppImage
# or for native:
buzz-desktop
```

- `WEBKIT_SKIA_ENABLE_CPU_RENDERING=1` forces Skia to use CPU rendering, bypassing the RDNA4 Skia/radv paint failure.
- `GDK_BACKEND=x11` avoids the blank window that appears when running under a Plasma-Wayland compositor.
- `WEBKIT_DISABLE_DMABUF_RENDERER=1` prevents post-first-paint transparency from the dmabuf renderer.

A dedicated fix for RDNA4 detection is being tracked in [#2643](https://github.com/block/buzz/issues/2643).

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
