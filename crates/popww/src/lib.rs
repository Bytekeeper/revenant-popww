//! popww: fixes for Prince of Persia: Warrior Within, loaded into `pop2.exe` by our launcher.

pub mod casefix;
#[cfg(windows)]
mod config;
#[cfg(windows)]
mod game;
#[cfg(windows)]
mod hardware;
#[cfg(windows)]
mod hooks;
#[cfg(windows)]
mod log;
#[cfg(windows)]
mod video;

#[cfg(windows)]
static MODULE_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// The directory of popww.dll, which is the game directory.
#[cfg(windows)]
fn module_dir() -> &'static std::path::Path {
    MODULE_DIR.get().map_or(std::path::Path::new("."), |dir| dir.as_path())
}

#[cfg(windows)]
mod entry {
    use std::ffi::c_void;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
    use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
    use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

    fn directory_of(module: HMODULE) -> PathBuf {
        let mut buf = [0u16; 1024];
        let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        let mut path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
        path.pop();
        path
    }

    /// Import-table anchor: the launcher adds an import of this function so the loader maps us.
    #[unsafe(no_mangle)]
    pub extern "C" fn popww_init() {}

    #[unsafe(no_mangle)]
    pub extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
        if reason == DLL_PROCESS_ATTACH {
            let _ = crate::MODULE_DIR.set(directory_of(module));
            crate::log::init();
            crate::log::log!("popww {} loaded", env!("CARGO_PKG_VERSION"));
            // Hooks go in before pop2.exe's UPX stub runs, so they also cover the unpacked game.
            crate::hooks::install_all();
            crate::video::install();
        }
        TRUE
    }
}
