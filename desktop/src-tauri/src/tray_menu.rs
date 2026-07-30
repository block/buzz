//! Native system-tray menu for the desktop app.
//!
//! The webview owns the live agent-turn state. It sends the small display
//! projection here so the native menu can remain useful while Buzz is hidden.

use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_foundation::{NSProcessInfo, NSString};
use serde::{Deserialize, Serialize};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    AppHandle, Emitter, Manager, Runtime,
};

const TRAY_ID: &str = "buzz-tray";
const OPEN_BUZZ_ID: &str = "tray-open-buzz";
const NEW_CHANNEL_ID: &str = "tray-new-channel";
const QUIT_ID: &str = "tray-quit";
const OPEN_CHANNEL_PREFIX: &str = "tray-open-channel:";
const OPEN_CHANNEL_ACTIVITY_SEPARATOR: char = '|';
#[cfg(target_os = "macos")]
const TRAY_MENU_MINIMUM_WIDTH: f64 = 320.0;

static PREVIEW_STARTED_AT: OnceLock<Instant> = OnceLock::new();

/// A local-only menu preview for demonstrating the working-agent section
/// without connecting to a relay. It is deliberately unavailable in release
/// builds and must be explicitly enabled when launching the debug app.
fn preview_activities() -> Option<Vec<TrayAgentActivity>> {
    if !cfg!(debug_assertions) || std::env::var("BUZZ_TRAY_MENU_DEMO").ok().as_deref() != Some("1")
    {
        return None;
    }

    let preview_elapsed = PREVIEW_STARTED_AT.get_or_init(Instant::now).elapsed();

    Some(vec![
        TrayAgentActivity {
            activity_id: "tray-preview-planning-scout".into(),
            agent_name: "Scout".into(),
            channel_id: "tray-preview-planning".into(),
            channel_name: "planning".into(),
            elapsed: format_elapsed(Duration::from_secs(192) + preview_elapsed),
        },
        TrayAgentActivity {
            activity_id: "tray-preview-planning-builder".into(),
            agent_name: "Builder".into(),
            channel_id: "tray-preview-planning".into(),
            channel_name: "planning".into(),
            elapsed: format_elapsed(Duration::from_secs(68) + preview_elapsed),
        },
        TrayAgentActivity {
            activity_id: "tray-preview-mobile-reviewer".into(),
            agent_name: "Reviewer".into(),
            channel_id: "tray-preview-mobile".into(),
            channel_name: "mobile".into(),
            elapsed: format_elapsed(Duration::from_secs(31) + preview_elapsed),
        },
    ])
}

fn preview_recent_activities() -> Option<Vec<TrayAgentActivity>> {
    if !cfg!(debug_assertions) || std::env::var("BUZZ_TRAY_MENU_DEMO").ok().as_deref() != Some("1")
    {
        return None;
    }

    Some(vec![TrayAgentActivity {
        activity_id: "recent:tray-preview-design-architect".into(),
        agent_name: "Architect".into(),
        channel_id: "tray-preview-design".into(),
        channel_name: "design".into(),
        elapsed: "4m 25s".into(),
    }])
}

fn format_elapsed(elapsed: Duration) -> String {
    let total_seconds = elapsed.as_secs();
    if total_seconds < 60 {
        return format!("{total_seconds}s");
    }

    let seconds = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    if total_minutes < 60 {
        return format!("{total_minutes}m {seconds}s");
    }

    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;
    format!("{hours}h {minutes}m {seconds}s")
}

/// Builds the standalone Buzz bee as a transparent, macOS template image.
///
/// The app icon includes a rounded square, which is useful for the Dock but
/// looks out of place beside the monochrome menu-bar icons. Keeping this
/// vector-derived mask here also lets macOS tint it correctly in light and
/// dark menu bars without a separate bitmap asset.
fn tray_bee_icon() -> Image<'static> {
    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 43;
    const SAMPLES_PER_AXIS: u32 = 4;
    const BEE_WIDTH: f32 = 466.0;
    const BEE_HEIGHT: f32 = 309.0;

    fn circle_contains(x: f32, y: f32, center_x: f32, center_y: f32, radius: f32) -> bool {
        let delta_x = x - center_x;
        let delta_y = y - center_y;
        delta_x * delta_x + delta_y * delta_y <= radius * radius
    }

    fn rounded_rect_contains(
        x: f32,
        y: f32,
        left: f32,
        top: f32,
        width: f32,
        height: f32,
        radius: f32,
    ) -> bool {
        let right = left + width;
        let bottom = top + height;
        let closest_x = x.clamp(left + radius, right - radius);
        let closest_y = y.clamp(top + radius, bottom - radius);
        let delta_x = x - closest_x;
        let delta_y = y - closest_y;
        delta_x * delta_x + delta_y * delta_y <= radius * radius
    }

    fn bee_contains(x: f32, y: f32) -> bool {
        let silhouette = circle_contains(x, y, 91.7, 154.5, 91.7)
            || circle_contains(x, y, 374.3, 154.5, 91.7)
            || rounded_rect_contains(x, y, 128.0, 0.0, 210.0, 309.0, 34.0);
        let cutout = circle_contains(x, y, 193.3, 84.4, 27.0)
            || circle_contains(x, y, 276.0, 84.4, 27.0)
            || rounded_rect_contains(x, y, 166.3, 157.2, 136.9, 38.3, 5.0)
            || rounded_rect_contains(x, y, 166.9, 235.1, 136.2, 37.6, 5.0);

        silhouette && !cutout
    }

    let mut rgba = vec![0; (WIDTH * HEIGHT * 4) as usize];
    let samples = SAMPLES_PER_AXIS * SAMPLES_PER_AXIS;

    for pixel_y in 0..HEIGHT {
        for pixel_x in 0..WIDTH {
            let mut covered_samples = 0;
            for sample_y in 0..SAMPLES_PER_AXIS {
                for sample_x in 0..SAMPLES_PER_AXIS {
                    let x = (pixel_x as f32 + (sample_x as f32 + 0.5) / SAMPLES_PER_AXIS as f32)
                        / WIDTH as f32
                        * BEE_WIDTH;
                    let y = (pixel_y as f32 + (sample_y as f32 + 0.5) / SAMPLES_PER_AXIS as f32)
                        / HEIGHT as f32
                        * BEE_HEIGHT;
                    if bee_contains(x, y) {
                        covered_samples += 1;
                    }
                }
            }

            let index = ((pixel_y * WIDTH + pixel_x) * 4) as usize;
            rgba[index + 3] = (covered_samples * u8::MAX as u32 / samples) as u8;
        }
    }

    Image::new_owned(rgba, WIDTH, HEIGHT)
}

/// A running agent and its current channel.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayAgentActivity {
    activity_id: String,
    agent_name: String,
    channel_id: String,
    channel_name: String,
    elapsed: String,
}

struct TrayActivityMenuItem<R: Runtime> {
    activity_id: String,
    channel_id: String,
    agent_item: MenuItem<R>,
}

struct TrayActionQueue {
    community_generation: u64,
    pending_actions: Vec<TrayAction>,
}

struct TrayMenuState<R: Runtime> {
    activity_items: Mutex<Vec<TrayActivityMenuItem<R>>>,
    action_queue: Mutex<TrayActionQueue>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TrayAction {
    NewChannel,
    OpenChannel {
        channel_id: String,
        community_generation: u64,
    },
}

pub(crate) fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(error) = window.unminimize() {
        eprintln!("buzz-desktop: failed to restore main window from tray: {error}");
        return;
    }
    if let Err(error) = window.show() {
        eprintln!("buzz-desktop: failed to show main window from tray: {error}");
        return;
    }
    if let Err(error) = window.set_focus() {
        eprintln!("buzz-desktop: failed to focus main window from tray: {error}");
    }
}

fn queue_tray_action<R: Runtime>(app: &AppHandle<R>, mut action: TrayAction) {
    let state = app.state::<TrayMenuState<R>>();
    let Ok(mut queue) = state.action_queue.lock() else {
        eprintln!("buzz-desktop: tray action queue is unavailable");
        return;
    };
    if let TrayAction::OpenChannel {
        community_generation,
        ..
    } = &mut action
    {
        *community_generation = queue.community_generation;
    }
    queue.pending_actions.push(action);
    drop(queue);

    if let Err(error) = app.emit("tray-action-available", ()) {
        eprintln!("buzz-desktop: failed to notify frontend of tray action: {error}");
    }
}

fn append_separator<R: Runtime>(app: &AppHandle<R>, menu: &Menu<R>) -> tauri::Result<()> {
    menu.append(&PredefinedMenuItem::separator(app)?)
}

fn agent_item_label(activity: &TrayAgentActivity) -> String {
    let primary = format!("{} · {}", activity.agent_name, activity.elapsed);

    #[cfg(target_os = "macos")]
    {
        if supports_menu_item_subtitles() {
            primary
        } else {
            format!("{primary} — #{}", activity.channel_name)
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        format!("{primary} — #{}", activity.channel_name)
    }
}

#[cfg(target_os = "macos")]
fn supports_menu_item_subtitles() -> bool {
    static SUPPORTS_SUBTITLES: OnceLock<bool> = OnceLock::new();
    *SUPPORTS_SUBTITLES.get_or_init(|| {
        NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= 14
    })
}

fn channel_item_id(activity: &TrayAgentActivity) -> String {
    format!(
        "{OPEN_CHANNEL_PREFIX}{}{OPEN_CHANNEL_ACTIVITY_SEPARATOR}{}",
        activity.channel_id, activity.activity_id
    )
}

fn build_menu<R: Runtime>(
    app: &AppHandle<R>,
    activities: &[TrayAgentActivity],
    recent_activities: &[TrayAgentActivity],
) -> tauri::Result<(Menu<R>, Vec<TrayActivityMenuItem<R>>)> {
    let menu = Menu::new(app)?;
    let mut activity_items =
        Vec::with_capacity(activities.len().saturating_add(recent_activities.len()));

    let running = MenuItem::new(app, "Running", false, None::<&str>)?;
    menu.append(&running)?;

    if activities.is_empty() {
        let empty = MenuItem::new(app, "No agents are running", false, None::<&str>)?;
        menu.append(&empty)?;
    } else {
        append_activity_items(app, &menu, activities, &mut activity_items)?;
    }

    if !recent_activities.is_empty() {
        append_separator(app, &menu)?;
        let recent = MenuItem::new(app, "Recent", false, None::<&str>)?;
        menu.append(&recent)?;
        append_activity_items(app, &menu, recent_activities, &mut activity_items)?;
    }

    append_separator(app, &menu)?;
    menu.append(&MenuItem::with_id(
        app,
        NEW_CHANNEL_ID,
        "New Channel",
        true,
        None::<&str>,
    )?)?;
    append_separator(app, &menu)?;
    menu.append(&MenuItem::with_id(
        app,
        OPEN_BUZZ_ID,
        "Open Buzz",
        true,
        None::<&str>,
    )?)?;
    append_separator(app, &menu)?;
    menu.append(&MenuItem::with_id(
        app,
        QUIT_ID,
        "Quit Buzz",
        true,
        None::<&str>,
    )?)?;

    Ok((menu, activity_items))
}

fn append_activity_items<R: Runtime>(
    app: &AppHandle<R>,
    menu: &Menu<R>,
    activities: &[TrayAgentActivity],
    activity_items: &mut Vec<TrayActivityMenuItem<R>>,
) -> tauri::Result<()> {
    for activity in activities {
        let agent_item = MenuItem::with_id(
            app,
            channel_item_id(activity),
            agent_item_label(activity),
            true,
            None::<&str>,
        )?;
        menu.append(&agent_item)?;
        activity_items.push(TrayActivityMenuItem {
            activity_id: activity.activity_id.clone(),
            channel_id: activity.channel_id.clone(),
            agent_item,
        });
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_activity_presentation<R: Runtime>(
    tray: &TrayIcon<R>,
    activities: &[TrayAgentActivity],
    recent_activities: &[TrayAgentActivity],
) -> Result<(), String> {
    if !supports_menu_item_subtitles() {
        return Ok(());
    }

    let subtitles = activities
        .iter()
        .chain(recent_activities)
        .map(|activity| format!("#{}", activity.channel_name))
        .collect::<Vec<_>>();
    let running_count = activities.len();
    let recent_count = recent_activities.len();

    tray.with_inner_tray_icon(move |inner| {
        let Some(status_item) = inner.ns_status_item() else {
            return;
        };
        let Some(main_thread) = MainThreadMarker::new() else {
            return;
        };
        let Some(menu) = status_item.menu(main_thread) else {
            return;
        };
        menu.setMinimumWidth(TRAY_MENU_MINIMUM_WIDTH);

        let mut item_index = 1;
        for subtitle in subtitles.iter().take(running_count) {
            if let Some(item) = menu.itemAtIndex(item_index) {
                let subtitle = NSString::from_str(subtitle);
                item.setSubtitle(Some(&subtitle));
            }
            item_index += 1;
        }

        if running_count == 0 {
            item_index += 1;
        }

        if recent_count > 0 {
            // The separator and Recent heading precede the completed rows.
            item_index += 2;
            for subtitle in subtitles.iter().skip(running_count) {
                if let Some(item) = menu.itemAtIndex(item_index) {
                    let subtitle = NSString::from_str(subtitle);
                    item.setSubtitle(Some(&subtitle));
                }
                item_index += 1;
            }
        }
    })
    .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "macos"))]
fn apply_activity_presentation<R: Runtime>(
    _tray: &TrayIcon<R>,
    _activities: &[TrayAgentActivity],
    _recent_activities: &[TrayAgentActivity],
) -> Result<(), String> {
    Ok(())
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    match id {
        OPEN_BUZZ_ID => show_main_window(app),
        NEW_CHANNEL_ID => {
            show_main_window(app);
            queue_tray_action(app, TrayAction::NewChannel);
        }
        QUIT_ID => app.exit(0),
        _ => {
            let Some(channel_id) = id.strip_prefix(OPEN_CHANNEL_PREFIX) else {
                return;
            };
            show_main_window(app);
            let channel_id = channel_id
                .split_once(OPEN_CHANNEL_ACTIVITY_SEPARATOR)
                .map(|(channel_id, _)| channel_id)
                .unwrap_or(channel_id);
            queue_tray_action(
                app,
                TrayAction::OpenChannel {
                    channel_id: channel_id.into(),
                    community_generation: 0,
                },
            );
        }
    }
}

/// Installs the persistent Buzz tray icon with the initial empty activity menu.
pub fn init<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let preview_activities = preview_activities();
    let preview_recent_activities = preview_recent_activities();
    let activities = preview_activities.as_deref().unwrap_or(&[]);
    let recent_activities = preview_recent_activities.as_deref().unwrap_or(&[]);
    let (menu, activity_items) = build_menu(app, activities, recent_activities)?;
    app.manage(TrayMenuState {
        activity_items: Mutex::new(activity_items),
        action_queue: Mutex::new(TrayActionQueue {
            community_generation: 0,
            pending_actions: Vec::new(),
        }),
    });
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .icon(tray_bee_icon())
        .icon_as_template(true)
        .on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref()))
        .build(app)?;
    if let Err(error) = apply_activity_presentation(&tray, activities, recent_activities) {
        eprintln!("buzz-desktop: failed to apply tray menu presentation: {error}");
    }
    Ok(())
}

/// Drains actions selected from the tray while the frontend was unavailable.
#[tauri::command]
pub fn take_tray_actions<R: Runtime>(app: AppHandle<R>) -> Result<Vec<TrayAction>, String> {
    let state = app.state::<TrayMenuState<R>>();
    let mut queue = state
        .action_queue
        .lock()
        .map_err(|_| "Buzz tray action queue is unavailable".to_string())?;
    Ok(std::mem::take(&mut queue.pending_actions))
}

fn requeue_actions(queue: &mut TrayActionQueue, mut actions: Vec<TrayAction>) {
    actions.retain(|action| match action {
        TrayAction::NewChannel => true,
        TrayAction::OpenChannel {
            community_generation,
            ..
        } => *community_generation == queue.community_generation,
    });
    actions.append(&mut queue.pending_actions);
    queue.pending_actions = actions;
}

/// Restores actions that were drained as the frontend unmounted. Channel
/// actions from a previous community generation are discarded.
#[tauri::command]
pub fn requeue_tray_actions<R: Runtime>(
    app: AppHandle<R>,
    actions: Vec<TrayAction>,
) -> Result<(), String> {
    let state = app.state::<TrayMenuState<R>>();
    let mut queue = state
        .action_queue
        .lock()
        .map_err(|_| "Buzz tray action queue is unavailable".to_string())?;
    requeue_actions(&mut queue, actions);
    drop(queue);
    app.emit("tray-action-available", ())
        .map_err(|error| error.to_string())
}

/// Clears community-scoped agent activity and queued channel navigation from
/// the native tray menu.
#[tauri::command]
pub fn clear_tray_agent_activity<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let state = app.state::<TrayMenuState<R>>();
    let mut queue = state
        .action_queue
        .lock()
        .map_err(|_| "Buzz tray action queue is unavailable".to_string())?;
    queue.community_generation = queue.community_generation.wrapping_add(1);
    queue
        .pending_actions
        .retain(|action| matches!(action, TrayAction::NewChannel));
    drop(queue);

    update_tray_agent_activity(app, Vec::new(), Vec::new())
}

/// Replaces the native menu's activity section with the current live work.
#[tauri::command]
pub fn update_tray_agent_activity<R: Runtime>(
    app: AppHandle<R>,
    activities: Vec<TrayAgentActivity>,
    recent_activities: Vec<TrayAgentActivity>,
) -> Result<(), String> {
    let preview_activities = preview_activities();
    let preview_recent_activities = preview_recent_activities();
    let activities = preview_activities.as_deref().unwrap_or(&activities);
    let recent_activities = preview_recent_activities
        .as_deref()
        .unwrap_or(&recent_activities);
    let state = app.state::<TrayMenuState<R>>();
    let mut activity_items = state
        .activity_items
        .lock()
        .map_err(|_| "Buzz tray menu state is unavailable".to_string())?;

    if activity_items.len() == activities.len().saturating_add(recent_activities.len())
        && activity_items
            .iter()
            .zip(activities.iter().chain(recent_activities))
            .all(|(item, activity)| {
                item.activity_id == activity.activity_id && item.channel_id == activity.channel_id
            })
    {
        for (item, activity) in activity_items
            .iter()
            .zip(activities.iter().chain(recent_activities))
        {
            item.agent_item
                .set_text(agent_item_label(activity))
                .map_err(|error| error.to_string())?;
        }
        let tray = app
            .tray_by_id(TRAY_ID)
            .ok_or_else(|| "Buzz tray icon is not available".to_string())?;
        apply_activity_presentation(&tray, activities, recent_activities)?;
        return Ok(());
    }

    let (menu, next_activity_items) =
        build_menu(&app, activities, recent_activities).map_err(|error| error.to_string())?;
    let tray = app
        .tray_by_id(TRAY_ID)
        .ok_or_else(|| "Buzz tray icon is not available".to_string())?;
    tray.set_menu(Some(menu))
        .map_err(|error| error.to_string())?;
    apply_activity_presentation(&tray, activities, recent_activities)?;
    *activity_items = next_activity_items;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{requeue_actions, TrayAction, TrayActionQueue};

    #[test]
    fn stale_channel_actions_are_not_requeued_after_community_change() {
        let mut queue = TrayActionQueue {
            community_generation: 2,
            pending_actions: Vec::new(),
        };

        requeue_actions(
            &mut queue,
            vec![TrayAction::OpenChannel {
                channel_id: "old-channel".into(),
                community_generation: 1,
            }],
        );

        assert!(queue.pending_actions.is_empty());
    }

    #[test]
    fn new_channel_actions_survive_community_change() {
        let mut queue = TrayActionQueue {
            community_generation: 2,
            pending_actions: Vec::new(),
        };

        requeue_actions(&mut queue, vec![TrayAction::NewChannel]);

        assert_eq!(queue.pending_actions, vec![TrayAction::NewChannel]);
    }
}
