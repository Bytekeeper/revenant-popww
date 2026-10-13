use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;

static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

/// Starts a fresh `popww.log` next to our DLL.
pub fn init(module: HMODULE) {
    let mut buf = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    let mut path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
    path.set_file_name("popww.log");
    if let Ok(file) = std::fs::File::create(path) {
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
