use crate::app_state::AppState;

#[tauri::command]
pub fn set_prevent_sleep_active(
    active: bool,
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    if active {
        crate::prevent_sleep::acquire(&state.prevent_sleep, &app_handle)
    } else {
        crate::prevent_sleep::release(&state.prevent_sleep);
        Ok(())
    }
}
