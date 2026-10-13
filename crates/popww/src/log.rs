use std::io::Write;
use std::sync::{Mutex, OnceLock};

static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

/// Starts a fresh `popww.log` next to our DLL.
pub fn init() {
    if let Ok(file) = std::fs::File::create(crate::module_dir().join("popww.log")) {
        let _ = LOG.set(Mutex::new(file));
    }
}

pub fn line(msg: std::fmt::Arguments) {
    if let Some(Ok(mut file)) = LOG.get().map(Mutex::lock) {
        let _ = writeln!(file, "{msg}");
    }
}

macro_rules! log {
    ($($arg:tt)*) => { $crate::log::line(format_args!($($arg)*)) };
}
pub(crate) use log;
