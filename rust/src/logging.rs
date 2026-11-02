use std::sync::atomic::{AtomicBool, Ordering};

static DEBUG: AtomicBool = AtomicBool::new(false);

pub fn init(debug: bool) {
    DEBUG.store(debug, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

pub fn debug(message: impl AsRef<str>) {
    if enabled() {
        eprintln!("[jb] {}", message.as_ref());
    }
}
