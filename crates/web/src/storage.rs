const LAST_CHECK_ID_KEY: &str = "very_debate_last_check_id";

pub fn load_last_check_id() -> Option<String> {
    let storage = web_sys::window()?.local_storage().ok()??;
    let value = storage.get_item(LAST_CHECK_ID_KEY).ok()??;
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub fn save_last_check_id(check_id: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(LAST_CHECK_ID_KEY, check_id);
    }
}

pub fn clear_last_check_id() {
    if let Some(storage) = local_storage() {
        let _ = storage.remove_item(LAST_CHECK_ID_KEY);
    }
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}
