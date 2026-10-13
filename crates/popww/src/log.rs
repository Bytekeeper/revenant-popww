use std::io::Write;
use std::sync::{Mutex, OnceLock};

static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
static START: OnceLock<std::time::Instant> = OnceLock::new();

/// Starts a fresh `popww.log` next to our DLL.
pub fn init() {
    START.get_or_init(std::time::Instant::now);
    if let Ok(file) = std::fs::File::create(crate::module_dir().join("popww.log")) {
        let _ = LOG.set(Mutex::new(file));
    }
}

pub fn line(msg: std::fmt::Arguments) {
    if let Some(Ok(mut file)) = LOG.get().map(Mutex::lock) {
        let elapsed = START.get().map_or(0, |start| start.elapsed().as_millis());
        let _ = writeln!(file, "{elapsed:>7} {msg}");
    }
}

macro_rules! log {
    ($($arg:tt)*) => { $crate::log::line(format_args!($($arg)*)) };
}
pub(crate) use log;
