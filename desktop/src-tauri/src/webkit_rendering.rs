//! WebKit rendering workarounds for Linux, applied before WebKit initializes.
//!
//! WebKitGTK's dmabuf renderer aborts the web process during startup on some
//! GPU/driver/compositor combinations, so Buzz comes up with no window at all
//! and the user has no way to fix it (#2338, upstream tauri#9394).
//!
//! Historically Buzz set `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which used to fall
//! back to shared-memory buffers. On current WebKitGTK that variable leaves the
//! transport mode empty, so `AcceleratedBackingStore::create()` returns null
//! and the UI SIGSEGVs the first time compositing is needed (#3654).
//! `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` is the documented replacement: it keeps
//! SharedMemory in the transport set and still avoids the hardware dmabuf path.
//!
//! WebKit reads each of these variables exactly once per process, so the choice
//! has to be made before anything initializes — there is no runtime toggle and
//! no second chance later in the same process. This module therefore decides
//! from cheap preflight signals instead of reacting to a crash:
//!
//! * an NVIDIA GPU, the driver family behind most upstream reports; and
//! * AppImage packaging, where linuxdeploy's AppRun hook pins `GDK_BACKEND=x11`
//!   and the dmabuf renderer buys nothing on that XWayland path (#2338).
//!
//! `--safe-rendering` is the manual escape hatch for a machine neither signal
//! recognises; it also disables accelerated compositing, for that launch only.
//!
//! This is the shape the Tauri ecosystem converged on: clash-verge-rev's
//! `utils/linux/workarounds.rs` and screenpipe's `linux_webkit_env.rs` both set
//! WebKit dmabuf env vars from the same signals at the same point in startup.

use std::ffi::{OsStr, OsString};
use std::path::Path;

/// Force the safest rendering configuration for this launch.
const SAFE_RENDERING: &str = "--safe-rendering";

/// PCI vendor ID reported by NVIDIA devices under `/sys/class/drm`.
const NVIDIA_PCI_VENDOR: &str = "0x10de";

/// Where DRM devices advertise their PCI vendor.
const DRM_ROOT: &str = "/sys/class/drm";

/// Prefer shared-memory dmabuf transport. The #3654 replacement for
/// `WEBKIT_DISABLE_DMABUF_RENDERER` on current WebKitGTK.
const FORCE_SHM: &str = "WEBKIT_DMABUF_RENDERER_FORCE_SHM";
/// Legacy kill-switch. Still owned so operators can set `=0` / `=1` and take
/// the decision away from this module, but never written by the heuristic
/// (it crashes modern WebKitGTK — see #3654).
const DISABLE_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
/// Drops accelerated compositing as well. `--safe-rendering` only.
const DISABLE_COMPOSITING: &str = "WEBKIT_DISABLE_COMPOSITING_MODE";

/// What the heuristic applies: force shared-memory transport without emptying
/// the buffer mode set (#3654).
const HEURISTIC: [&str; 1] = [FORCE_SHM];

/// What `--safe-rendering` applies: FORCE_SHM plus compositing off. Deliberately
/// omits DISABLE_DMABUF — that variable is the #3654 crash on current WebKit.
const SAFE_VARS: [&str; 2] = [FORCE_SHM, DISABLE_COMPOSITING];

/// Every variable this module may set, and therefore every variable a user
/// assignment takes away from it. Being the same list is the invariant: nothing
/// outside it is ever written, so a user value for any other WebKit variable is
/// not a conflict. DISABLE_DMABUF stays owned so `=0` still stands the heuristic
/// down for operators who need the old path or an explicit override.
const OWNED: [&str; 3] = [FORCE_SHM, DISABLE_DMABUF, DISABLE_COMPOSITING];

/// Reads one environment variable. Injected so the decision is testable without
/// mutating the process environment. `OsString` rather than `String` because
/// presence is the test — a non-UTF-8 assignment is still the user's.
type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// What this launch should do about its rendering environment.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Set each of these to `1`, then report `why`.
    Apply {
        vars: &'static [&'static str],
        why: String,
    },
    /// Change nothing, and report `why`.
    Leave { why: String },
    /// The request cannot be delivered. Report it and exit non-zero rather than
    /// starting an app that silently ignores what the user asked for.
    Fatal { diagnostic: String },
}

/// Applies the workaround for this launch.
///
/// Must be called from `main()` before `crate::run()`: WebKit memoizes these
/// variables at process start, and `std::env::set_var` is only sound while the
/// process is still single threaded, which it is nowhere else in Buzz.
///
/// `Err` carries a user-facing diagnostic; the caller reports it and exits.
pub fn apply() -> Result<(), String> {
    match plan(
        std::env::args_os(),
        &|key| std::env::var_os(key),
        Path::new(DRM_ROOT),
    ) {
        Plan::Apply { vars, why } => {
            for var in vars {
                // Safe here and only here — see the doc comment above.
                std::env::set_var(var, "1");
            }
            let applied: Vec<String> = vars.iter().map(|var| format!("{var}=1")).collect();
            eprintln!("buzz-desktop: {} — {why}", applied.join(" "));
            Ok(())
        }
        Plan::Leave { why } => {
            eprintln!("buzz-desktop: WebKit rendering left as-is — {why}");
            Ok(())
        }
        Plan::Fatal { diagnostic } => Err(diagnostic),
    }
}

/// The whole decision, as a pure function of argv, the environment, and the DRM
/// device tree.
fn plan(
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    env: EnvLookup<'_>,
    drm_root: &Path,
) -> Plan {
    let safe_rendering = args
        .into_iter()
        .any(|arg| arg.as_ref() == OsStr::new(SAFE_RENDERING));
    let user_set = user_set(env);

    if !user_set.is_empty() {
        // A user who has assigned one of these has taken over the decision, so
        // the heuristic stands down wholesale — writing the *other* variable
        // behind their back would be exactly the surprise they opted out of.
        return match safe_rendering {
            // Two incompatible answers to one question, and no basis for
            // picking: honouring the flag would overwrite configuration the
            // user typed, honouring the environment would silently ignore a
            // rescue flag from a user whose app does not start.
            true => Plan::Fatal {
                diagnostic: conflict(&user_set),
            },
            false => {
                let mut why = format!("{} set in the environment", describe(&user_set));
                // Older docs told people to export DISABLE_DMABUF=1; on current
                // WebKitGTK that empties the transport and SIGSEGVs (#3654).
                // Leave the takeover alone, but point survivors at FORCE_SHM.
                if user_set
                    .iter()
                    .any(|(key, value)| *key == DISABLE_DMABUF && value.as_os_str() != "0")
                {
                    why.push_str(&format!(
                        "; warning: {DISABLE_DMABUF} (other than =0) empties the \
                         transport on current WebKitGTK and SIGSEGVs — prefer \
                         {FORCE_SHM}=1 (see #3654)"
                    ));
                }
                Plan::Leave { why }
            }
        };
    }

    if safe_rendering {
        return Plan::Apply {
            vars: &SAFE_VARS,
            why: format!("{SAFE_RENDERING} requested, this launch only"),
        };
    }

    let signals = [
        (nvidia_gpu(drm_root), "NVIDIA GPU"),
        (env("APPIMAGE").is_some(), "AppImage"),
    ];
    let hits: Vec<&str> = signals
        .iter()
        .filter_map(|(hit, label)| hit.then_some(*label))
        .collect();

    match hits.is_empty() {
        true => Plan::Leave {
            why: "no NVIDIA GPU and not an AppImage".to_string(),
        },
        false => Plan::Apply {
            vars: &HEURISTIC,
            why: hits.join(", "),
        },
    }
}

/// Owned variables the environment already carries, keyed by name.
///
/// Presence is the test, not truthiness: `VAR=0` and `VAR=` are both genuine
/// user assignments, and both take the decision away from this module.
fn user_set(env: EnvLookup<'_>) -> Vec<(&'static str, OsString)> {
    OWNED
        .iter()
        .filter_map(|key| env(key).map(|value| (*key, value)))
        .collect()
}

/// User assignments rendered as `KEY=value`, for a log line or a diagnostic.
fn describe(user_set: &[(&str, OsString)]) -> String {
    let shown: Vec<String> = user_set
        .iter()
        .map(|(key, value)| format!("{key}={}", value.to_string_lossy()))
        .collect();
    shown.join(", ")
}

/// Whether any DRM device reports NVIDIA's PCI vendor ID. An unreadable device
/// tree is not a hit — the workaround has a real cost, so it needs evidence.
fn nvidia_gpu(drm_root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(drm_root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        std::fs::read_to_string(entry.path().join("device/vendor"))
            .is_ok_and(|vendor| vendor.trim().eq_ignore_ascii_case(NVIDIA_PCI_VENDOR))
    })
}

/// The diagnostic for `--safe-rendering` against a user-set owned variable.
///
/// The message both shows what is set and names the keys to unset — the two
/// things a user whose app will not start needs in order to act on it.
fn conflict(user_set: &[(&str, OsString)]) -> String {
    let keys: Vec<&str> = user_set.iter().map(|(key, _)| *key).collect();
    format!(
        "{SAFE_RENDERING} cannot be applied: {} already set in the environment. \
         Either unset {} and run {SAFE_RENDERING} again, or keep that \
         environment and drop the flag.",
        describe(user_set),
        keys.join(", "),
    )
}

#[cfg(test)]
mod tests;
