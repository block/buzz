//! The macOS application menu.
//!
//! Buzz never called `Builder::menu()`, so Tauri installed `Menu::default()`
//! for us (`tauri::app::Builder::build`, macOS arm). That default puts a
//! `close_window` item in both the File and Window submenus, and muda gives
//! that item a Cmd+W key equivalent bound to `performClose:`.
//!
//! That default cannot express Buzz's context-dependent behavior:
//!
//! 1. `CloseRequested` on the main window is intercepted in `lib.rs` and turned
//!    into hide-to-tray. Cmd+W should take that path in normal Buzz mode.
//! 2. macOS resolves a menu key equivalent before the webview receives any key
//!    event, so Buzz Term could never bind Cmd+W to "close this terminal tab"
//!    while the accelerator was claimed here.
//!
//! So this module builds the standard menu minus both `close_window` items.
//! Everything else matches `Menu::default()` deliberately. The webview routes
//! Cmd+W conditionally instead: Buzz Term consumes it in capture phase while
//! it owns input, and `useCloseWindowShortcut` closes the current window in
//! normal Buzz mode.

#[cfg(target_os = "macos")]
use tauri::menu::{
    AboutMetadata, Menu, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID,
};
use tauri::AppHandle;
use tauri::{Builder, Runtime};

/// The languages the native menu can be built in.
///
/// A command argument, so it is the trust boundary for the menu: the frontend
/// may name `en` or `zh-Hans` and nothing else. Deserialization rejects every
/// other string — including other locales, `null`, and objects — so a
/// compromised or buggy webview cannot feed arbitrary text into the native
/// menu, and Windows/Linux are never handed a locale to pretend to support.
///
/// Kept cross-platform on purpose: the command is registered on every host
/// (Tauri resolves handler lists at build time), so the type must exist even
/// where rebuilding a menu does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppMenuLocale {
    /// English — the language this menu shipped in before localization.
    #[serde(rename = "en")]
    En,
    /// Simplified Chinese, spelled exactly as the frontend's `AppLanguage`.
    #[serde(rename = "zh-Hans")]
    ZhHans,
}

/// Titles of the submenus Buzz names itself.
///
/// Everything else is a `PredefinedMenuItem`, which muda already resolves
/// against the operating system's language — those need no table here, and
/// adding one would let Buzz disagree with the system about "Copy"/"拷贝".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AppMenuTitles {
    edit: &'static str,
    view: &'static str,
    window: &'static str,
    help: &'static str,
}

const EN_TITLES: AppMenuTitles = AppMenuTitles {
    edit: "Edit",
    view: "View",
    window: "Window",
    help: "Help",
};

/// macOS's own Simplified-Chinese menu bar wording (`显示` for View, as
/// AppKit titles it, not the literal translation `视图`).
const ZH_HANS_TITLES: AppMenuTitles = AppMenuTitles {
    edit: "编辑",
    view: "显示",
    window: "窗口",
    help: "帮助",
};

impl AppMenuTitles {
    /// The title table for one menu locale.
    ///
    /// Read by `build`, which only exists on macOS; the table itself is not
    /// platform-gated so the enum and its wording stay unit-testable on the
    /// Windows and Linux hosts that run this file's tests.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn for_locale(locale: AppMenuLocale) -> Self {
        match locale {
            AppMenuLocale::En => EN_TITLES,
            AppMenuLocale::ZhHans => ZH_HANS_TITLES,
        }
    }
}

/// Installs Buzz's menu, replacing the `Menu::default()` Tauri would otherwise
/// auto-install. A no-op off macOS, where that default is never created and
/// the Cmd+W accelerator does not exist.
///
/// The boot menu is English because Rust has no catalog: `main.tsx` calls
/// `initializeI18n`, which immediately pushes the effective language through
/// [`set_app_menu_locale`]. A non-English user therefore sees one menu swap at
/// startup instead of a wrong-language menu they have to trigger.
pub fn install<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    #[cfg(target_os = "macos")]
    let builder = builder.menu(|app: &AppHandle<R>| build(app, AppMenuLocale::En));
    builder
}

/// Rebuilds the native application menu in `locale` (spec FR-008 / AC-009).
///
/// Off macOS there is no application menu for Buzz to own — Windows and Linux
/// keep their menu-less frame, and any system-provided menu or dialog follows
/// the OS language rather than the app's, which is out of reach here — so the
/// command answers success without touching anything.
#[tauri::command]
pub fn set_app_menu_locale<R: Runtime>(
    app: AppHandle<R>,
    locale: AppMenuLocale,
) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let menu = build(&app, locale)?;
        // `Option<Menu<R>>` is the menu AppKit replaced, if there was one.
        let _ = menu.set_as_app_menu()?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, locale);
    }
    Ok(())
}

/// Mirrors `Menu::default()` with every `close_window` item omitted.
///
/// The Window and Help submenus keep Tauri's well-known ids: `init_app_menu`
/// looks them up by id to call `set_as_windows_menu_for_nsapp` and
/// `set_as_help_menu_for_nsapp`, and a plain `with_items` submenu would skip
/// both silently -- no error, just a Window menu AppKit no longer manages.
#[cfg(target_os = "macos")]
pub fn build<R: Runtime>(app: &AppHandle<R>, locale: AppMenuLocale) -> tauri::Result<Menu<R>> {
    let titles = AppMenuTitles::for_locale(locale);
    let pkg_info = app.package_info();
    let config = app.config();
    let about_metadata = AboutMetadata {
        name: Some(pkg_info.name.clone()),
        version: Some(pkg_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };

    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                pkg_info.name.clone(),
                true,
                &[
                    &PredefinedMenuItem::about(app, None, Some(about_metadata))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            // `Menu::default()`'s File submenu holds exactly one item on macOS
            // -- close_window -- so dropping that item drops the submenu too.
            &Submenu::with_items(
                app,
                titles.edit,
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                titles.view,
                true,
                &[&PredefinedMenuItem::fullscreen(app, None)?],
            )?,
            &Submenu::with_id_and_items(
                app,
                WINDOW_SUBMENU_ID,
                titles.window,
                true,
                &[
                    &PredefinedMenuItem::minimize(app, None)?,
                    &PredefinedMenuItem::maximize(app, None)?,
                ],
            )?,
            // Empty upstream too on macOS: About lives in the app submenu.
            &Submenu::with_id_and_items(app, HELP_SUBMENU_ID, titles.help, true, &[])?,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::{AppMenuLocale, AppMenuTitles};

    fn decode(wire: &str) -> Option<AppMenuLocale> {
        serde_json::from_value(serde_json::Value::String(wire.into())).ok()
    }

    #[test]
    fn the_command_accepts_exactly_the_two_shipped_languages() {
        assert_eq!(decode("en"), Some(AppMenuLocale::En));
        assert_eq!(decode("zh-Hans"), Some(AppMenuLocale::ZhHans));
    }

    #[test]
    fn any_other_locale_is_rejected_before_it_reaches_the_menu() {
        // The webview speaks `zh-Hans`; `zh-CN` is a system tag, not a catalog
        // name, and the rest are the shapes a confused or hostile caller sends.
        for rejected in [
            "zh-CN", "zh", "zh-Hant", "en-US", "EN", "French", "", "delete",
        ] {
            assert_eq!(decode(rejected), None, "accepted {rejected:?}");
        }
    }

    #[test]
    fn every_locale_names_all_four_submenus() {
        for locale in [AppMenuLocale::En, AppMenuLocale::ZhHans] {
            let titles = AppMenuTitles::for_locale(locale);
            for title in [titles.edit, titles.view, titles.window, titles.help] {
                assert!(
                    !title.trim().is_empty(),
                    "blank submenu title for {locale:?}"
                );
            }
        }
        assert_eq!(
            AppMenuTitles::for_locale(AppMenuLocale::En),
            AppMenuTitles {
                edit: "Edit",
                view: "View",
                window: "Window",
                help: "Help",
            },
            "the English menu must not drift from what Buzz shipped before localization"
        );
    }
}
