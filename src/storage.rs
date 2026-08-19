//! Persistence for the interactive UI.

use serde::{Deserialize, Serialize};

const UI_STATE_KEY: &str = "vello_bench_ui_state";
const BACKEND_KEY: &str = "vello_bench_renderer";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct UiState {
    pub(crate) sidebar_collapsed: Option<bool>,
    pub(crate) scene: Option<usize>,
    #[serde(default)]
    pub(crate) params: Vec<(String, f64)>,
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub(crate) fn load_ui_state() -> UiState {
    let Some(storage) = local_storage() else {
        return UiState::default();
    };
    let Some(json) = storage.get_item(UI_STATE_KEY).ok().flatten() else {
        return UiState::default();
    };
    serde_json::from_str(&json).unwrap_or_default()
}

pub(crate) fn save_ui_state(state: &UiState) {
    if let Some(storage) = local_storage()
        && let Ok(json) = serde_json::to_string(state)
    {
        let _ = storage.set_item(UI_STATE_KEY, &json);
    }
}

pub(crate) fn load_backend_name() -> Option<String> {
    local_storage()?.get_item(BACKEND_KEY).ok().flatten()
}

pub(crate) fn save_backend_name(name: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(BACKEND_KEY, name);
    }
}
