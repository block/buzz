//! WebKit rendering workarounds for Linux, applied before WebKit initializes.
//!
//! WebKitGTK's dmabuf renderer aborts the web process during startup on some
//! GPU/driver/compositor combinations, so Buzz comes up with no window at all
//! and the user has no way to fix it (#2338, upstream tauri#9394). Setting
//! `WEBKIT_DISABLE_DMABUF_RENDERER=1` avoids the abort by falling back to the
//! shared-memory buffer path.
//!
//! WebKit reads each of these variables exactly once per process, so the choice
//! has to be made before anything initializes — there is no runtime toggle and
//! no second chance later in the same process. This module therefore decides
//! from cheap preflight signals instead of reacting to a crash:
//!
//! * an NVIDIA GPU, the driver family behind most upstream reports; and
//! * AppImage packaging, where linuxdeploy's AppRun hook pins `GDK_BACKEND=x11`
//!   and the dmabuf renderer buys nothing on that XWayland path (#2338); and
//! * an x86_64 CPU without AVX, where affected WebKitGTK 2.52 JavaScriptCore
//!   builds can execute an AVX instruction and kill WebKitWebProcess (#3747).
//!
//! `--safe-rendering` is the manual escape hatch for a machine neither signal
//! recognises; it also disables accelerated compositing, for that launch only.
//!
//! This is the shape the Tauri ecosystem converged on: clash-verge-rev's
//! `utils/linux/workarounds.rs` and screenpipe's `linux_webkit_env.rs` both set
//! the same variable from the same signals at the same point in startup.

use std::ffi::{OsStr, OsString};
use std::path::Path;

/// Force the safest rendering configuration for this launch.
const SAFE_RENDERING: &str = "--safe-rendering";

/// PCI vendor ID reported by NVIDIA devices under `/sys/class/drm`.
const NVIDIA_PCI_VENDOR: &str = "0x10de";

/// Where DRM devices advertise their PCI vendor.
const DRM_ROOT: &str = "/sys/class/drm";
/// Linux's processor feature inventory.
const CPU_INFO: &str = "/proc/cpuinfo";

/// Drops the zero-copy dmabuf buffer path. The workaround for #2338.
const DISABLE_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
/// Drops accelerated compositing as well. `--safe-rendering` only.
const DISABLE_COMPOSITING: &str = "WEBKIT_DISABLE_COMPOSITING_MODE";
/// Disables JavaScriptCore's JIT on AVX-less x86_64 CPUs. The workaround for
/// #3747.
const DISABLE_JSC_JIT: &str = "JSC_useJIT";

/// What the heuristic applies: the #2338 workaround alone, matching the
/// ecosystem precedents. `DISABLE_COMPOSITING` is deliberately not here — no
/// report has isolated it as necessary, and it costs more rendering than this.
const HEURISTIC: [(&str, &str); 1] = [(DISABLE_DMABUF, "1")];

/// What `--safe-rendering` applies, which is also every rendering variable a
/// user assignment takes away from this module. The JSC workaround is an
/// independent decision with its own user override.
const OWNED: [(&str, &str); 2] = [(DISABLE_DMABUF, "1"), (DISABLE_COMPOSITING, "1")];

/// Reads one environment variable. Injected so the decision is testable without
/// mutating the process environment. `OsString` rather than `String` because
/// presence is the test — a non-UTF-8 assignment is still the user's.
type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// What this launch should do about its rendering environment.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Apply each `(variable, value)` assignment, then report `why`.
    Apply {
        assignments: Vec<(&'static str, &'static str)>,
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
        std::env::consts::ARCH,
        Path::new(CPU_INFO),
    ) {
        Plan::Apply { assignments, why } => {
            for (var, value) in &assignments {
                // Safe here and only here — see the doc comment above.
                std::env::set_var(var, value);
            }
            let applied: Vec<String> = assignments
                .iter()
                .map(|(var, value)| format!("{var}={value}"))
                .collect();
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

/// The whole decision, as a function of injected argv, environment, platform,
/// and Linux hardware inventory paths.
fn plan(
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    env: EnvLookup<'_>,
    drm_root: &Path,
    arch: &str,
    cpu_info: &Path,
) -> Plan {
    let safe_rendering = args
        .into_iter()
        .any(|arg| arg.as_ref() == OsStr::new(SAFE_RENDERING));
    let user_set = user_set(env);

    if !user_set.is_empty() {
        // A user who has assigned one of these has taken over the decision, so
        // the heuristic stands down wholesale — writing the *other* variable
        // behind their back would be exactly the surprise they opted out of.
        if safe_rendering {
            // Two incompatible answers to one question, and no basis for
            // picking: honouring the flag would overwrite configuration the
            // user typed, honouring the environment would silently ignore a
            // rescue flag from a user whose app does not start.
            return Plan::Fatal {
                diagnostic: conflict(&user_set),
            };
        }
    }

    let mut assignments = Vec::new();
    let mut reasons = Vec::new();

    if safe_rendering {
        assignments.extend(OWNED);
        reasons.push(format!("{SAFE_RENDERING} requested, this launch only"));
    } else if user_set.is_empty() {
        let signals = [
            (nvidia_gpu(drm_root), "NVIDIA GPU"),
            (env("APPIMAGE").is_some(), "AppImage"),
        ];
        let hits: Vec<&str> = signals
            .iter()
            .filter_map(|(hit, label)| hit.then_some(*label))
            .collect();
        if !hits.is_empty() {
            assignments.extend(HEURISTIC);
            reasons.push(hits.join(", "));
        }
    } else {
        reasons.push(format!("{} set in the environment", describe(&user_set)));
    }

    if env(DISABLE_JSC_JIT).is_some() {
        reasons.push(format!("{DISABLE_JSC_JIT} set in the environment"));
    } else if avx_available(arch, cpu_info) == Some(false) {
        assignments.push((DISABLE_JSC_JIT, "0"));
        reasons.push("x86_64 CPU does not advertise AVX".to_string());
    }

    match assignments.is_empty() {
        true => Plan::Leave {
            why: match reasons.is_empty() {
                true => "no NVIDIA GPU, not an AppImage, and no AVX-less x86_64 CPU detected"
                    .to_string(),
                false => reasons.join("; "),
            },
        },
        false => Plan::Apply {
            assignments,
            why: reasons.join("; "),
        },
    }
}

/// Whether an x86_64 CPU advertises AVX. Unknown architecture or unreadable /
/// malformed CPU data produces no decision rather than disabling the JIT.
fn avx_available(arch: &str, cpu_info: &Path) -> Option<bool> {
    if arch != "x86_64" {
        return None;
    }

    let cpu_info = std::fs::read_to_string(cpu_info).ok()?;
    cpu_info.lines().find_map(|line| {
        let (key, features) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case("flags").then(|| {
            features
                .split_ascii_whitespace()
                .any(|feature| feature.eq_ignore_ascii_case("avx"))
        })
    })
}

/// Owned variables the environment already carries, keyed by name.
///
/// Presence is the test, not truthiness: `VAR=0` and `VAR=` are both genuine
/// user assignments, and both take the decision away from this module.
fn user_set(env: EnvLookup<'_>) -> Vec<(&'static str, OsString)> {
    OWNED
        .iter()
        .filter_map(|&(key, _)| env(key).map(|value| (key, value)))
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
