use std::{env, sync::LazyLock};

pub fn is_debug() -> bool {
    static IS_DEBUG: LazyLock<bool> = LazyLock::new(|| {
        env::var("CONTAINERYARD_DEBUG")
            .map(|v| v != "0")
            .unwrap_or(false)
    });
    IS_DEBUG.clone()
}
