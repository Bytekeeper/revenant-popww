//! popww: fixes for Prince of Persia: Warrior Within, loaded into `pop2.exe` by our launcher.

pub mod casefix;
#[cfg(windows)]
mod hardware;
#[cfg(windows)]
mod hooks;
#[cfg(windows)]
mod log;

#[cfg(windows)]
mod entry {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
    use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

    /// Import-table anchor: the launcher adds an import of this function so the loader maps us.
    #[unsafe(no_mangle)]
    pub extern "C" fn popww_init() {}

    #[unsafe(no_mangle)]
    pub extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
        if reason == DLL_PROCESS_ATTACH {
            crate::log::init(module);
            crate::log::log!("popww {} loaded", env!("CARGO_PKG_VERSION"));
            // Hooks go in before pop2.exe's UPX stub runs, so they also cover the unpacked game.
            crate::hooks::install_all();
        }
        TRUE
    }
}
