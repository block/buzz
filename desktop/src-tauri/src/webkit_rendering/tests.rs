//! Behaviour of the preflight decision.
//!
//! Every case goes through `plan`, which takes argv, the environment, and the
//! DRM root as arguments — so nothing here mutates the process environment and
//! the tests are order-independent.

use super::*;

const NO_ARGS: [&str; 0] = [];

/// A `/sys/class/drm` stand-in. `vendors` are written as `card<n>/device/vendor`
/// with the trailing newline the kernel emits.
fn drm(vendors: &[&str]) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("tempdir");
    for (index, vendor) in vendors.iter().enumerate() {
        let device = root.path().join(format!("card{index}")).join("device");
        std::fs::create_dir_all(&device).expect("device dir");
        std::fs::write(device.join("vendor"), format!("{vendor}\n")).expect("vendor");
    }
    root
}

fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let owned: Vec<(String, OsString)> = pairs
        .iter()
        .map(|(key, value)| (key.to_string(), OsString::from(value)))
        .collect();
    move |key| {
        owned
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.clone())
    }
}

/// The variables a plan would set, or `None` for a plan that sets nothing.
fn applied(plan: &Plan) -> Option<&[&str]> {
    match plan {
        Plan::Apply { vars, .. } => Some(vars),
        _ => None,
    }
}

// ── Detection ───────────────────────────────────────────────────────────────

#[test]
fn test_nvidia_gpu_disables_the_dmabuf_renderer() {
    let drm = drm(&["0x10de"]);
    let plan = plan(NO_ARGS, &env_from(&[]), drm.path());

    assert_eq!(
        applied(&plan),
        Some(&["WEBKIT_DISABLE_DMABUF_RENDERER"][..])
    );
    let Plan::Apply { why, .. } = &plan else {
        unreachable!()
    };
    assert!(why.contains("NVIDIA"), "{why}");
}

#[test]
fn test_an_nvidia_gpu_alongside_another_vendor_still_counts() {
    // Hybrid graphics: the integrated GPU enumerates first, and WebKit may
    // still land on the discrete one.
    let drm = drm(&["0x8086", "0x10de"]);

    assert_eq!(
        applied(&plan(NO_ARGS, &env_from(&[]), drm.path())),
        Some(&["WEBKIT_DISABLE_DMABUF_RENDERER"][..])
    );
}

#[test]
fn test_the_vendor_id_match_ignores_case() {
    let drm = drm(&["0x10DE"]);

    assert_eq!(
        applied(&plan(NO_ARGS, &env_from(&[]), drm.path())),
        Some(&["WEBKIT_DISABLE_DMABUF_RENDERER"][..])
    );
}

#[test]
fn test_an_appimage_launch_disables_the_dmabuf_renderer() {
    // No NVIDIA GPU: the AppImage signal has to carry this on its own, which is
    // #2338's reporter (Intel Mesa under the AppRun's pinned XWayland backend).
    let drm = drm(&["0x8086"]);
    let env = env_from(&[("APPIMAGE", "/home/u/Buzz.AppImage")]);
    let plan = plan(NO_ARGS, &env, drm.path());

    assert_eq!(
        applied(&plan),
        Some(&["WEBKIT_DISABLE_DMABUF_RENDERER"][..])
    );
    let Plan::Apply { why, .. } = &plan else {
        unreachable!()
    };
    assert!(why.contains("AppImage"), "{why}");
}

#[test]
fn test_a_plain_non_nvidia_launch_changes_nothing() {
    let drm = drm(&["0x8086", "0x1002"]);

    assert!(matches!(
        plan(NO_ARGS, &env_from(&[]), drm.path()),
        Plan::Leave { .. }
    ));
}

#[test]
fn test_an_unreadable_drm_tree_is_not_treated_as_a_hit() {
    // Containers and hardened kernels can hide `/sys/class/drm` entirely. The
    // workaround costs real rendering performance, so absent evidence is not
    // evidence — this must not become an unconditional export.
    let missing = std::path::Path::new("/nonexistent/class/drm");

    assert!(matches!(
        plan(NO_ARGS, &env_from(&[]), missing),
        Plan::Leave { .. }
    ));
}

#[test]
fn test_a_device_without_a_vendor_file_is_skipped_not_fatal() {
    // `/sys/class/drm` also contains connector entries (`card0-HDMI-A-1`) and
    // `renderD*` nodes, which have no `device/vendor` under them.
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(root.path().join("card0-HDMI-A-1")).expect("connector");
    let device = root.path().join("card1").join("device");
    std::fs::create_dir_all(&device).expect("device dir");
    std::fs::write(device.join("vendor"), "0x10de\n").expect("vendor");

    assert_eq!(
        applied(&plan(NO_ARGS, &env_from(&[]), root.path())),
        Some(&["WEBKIT_DISABLE_DMABUF_RENDERER"][..])
    );
}

// ── User environment ────────────────────────────────────────────────────────

#[test]
fn test_a_user_set_variable_disables_the_heuristic_wholesale() {
    // `0` is the value a truthiness check would drop: the user is asking for the
    // dmabuf renderer *on*, on a machine the heuristic would have opted out.
    let drm = drm(&["0x10de"]);
    let env = env_from(&[(DISABLE_DMABUF, "0")]);
    let plan = plan(NO_ARGS, &env, drm.path());

    let Plan::Leave { why } = &plan else {
        panic!("a user assignment must not be overwritten: {plan:?}");
    };
    assert!(why.contains("WEBKIT_DISABLE_DMABUF_RENDERER=0"), "{why}");
}

#[test]
fn test_an_empty_assignment_is_still_a_user_assignment() {
    let drm = drm(&["0x10de"]);
    let env = env_from(&[(DISABLE_DMABUF, "")]);

    assert!(matches!(
        plan(NO_ARGS, &env, drm.path()),
        Plan::Leave { .. }
    ));
}

#[test]
fn test_a_user_set_compositing_variable_also_stands_the_heuristic_down() {
    // The heuristic never sets this one, but it is still ours to set under
    // `--safe-rendering`, so a user value takes the whole decision away rather
    // than leaving us free to write the sibling variable.
    let drm = drm(&["0x10de"]);
    let env = env_from(&[(DISABLE_COMPOSITING, "1")]);

    assert!(matches!(
        plan(NO_ARGS, &env, drm.path()),
        Plan::Leave { .. }
    ));
}

// ── --safe-rendering ────────────────────────────────────────────────────────

#[test]
fn test_safe_rendering_applies_the_safest_set_without_any_hardware_signal() {
    // The escape hatch exists for the machine neither signal recognises, so it
    // must not depend on either one.
    let drm = drm(&["0x8086"]);
    let args = ["buzz://channel/1", SAFE_RENDERING];
    let plan = plan(args, &env_from(&[]), drm.path());

    assert_eq!(
        applied(&plan),
        Some(
            &[
                "WEBKIT_DISABLE_DMABUF_RENDERER",
                "WEBKIT_DISABLE_COMPOSITING_MODE"
            ][..]
        )
    );
}

#[test]
fn test_an_unrelated_flag_is_not_mistaken_for_safe_rendering() {
    let drm = drm(&["0x8086"]);

    assert!(matches!(
        plan(["--safe-renderingX"], &env_from(&[]), drm.path()),
        Plan::Leave { .. }
    ));
}

#[test]
fn test_safe_rendering_against_a_user_set_variable_is_fatal_not_guessed() {
    let drm = drm(&["0x8086"]);
    let env = env_from(&[(DISABLE_DMABUF, "0")]);
    let plan = plan([SAFE_RENDERING], &env, drm.path());

    let Plan::Fatal { diagnostic } = &plan else {
        panic!("the flag and the environment disagree; neither may be guessed: {plan:?}");
    };
    // The message has to name what is set and what to unset, or the user whose
    // app will not start cannot act on it.
    assert!(diagnostic.contains(SAFE_RENDERING), "{diagnostic}");
    assert!(
        diagnostic.contains("WEBKIT_DISABLE_DMABUF_RENDERER=0"),
        "{diagnostic}"
    );
}

#[test]
fn test_a_non_utf8_user_assignment_is_reported_not_ignored() {
    // Presence is the test, so this still stands the heuristic down; the
    // diagnostic must name the key rather than dropping the whole entry.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        let drm = drm(&["0x10de"]);
        let invalid = OsString::from_vec(vec![0xff, 0xfe]);
        let env = |key: &str| match key == DISABLE_DMABUF {
            true => Some(invalid.clone()),
            false => None,
        };

        let Plan::Fatal { diagnostic } = plan([SAFE_RENDERING], &env, drm.path()) else {
            panic!("a non-UTF-8 assignment is still a user assignment");
        };
        assert!(diagnostic.contains(DISABLE_DMABUF), "{diagnostic}");
    }
}
