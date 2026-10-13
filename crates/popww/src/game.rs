//! Patches and hooks inside `pop2.exe` (GOG v1.0.0.188 "pop5_v189").
//!
//! The code only exists after the UPX stub has unpacked it, so everything here waits for the C
//! runtime's first `GetCommandLineA` and checks the original bytes before touching anything.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use minhook::MinHook;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, PAGE_GUARD, PAGE_NOACCESS, VirtualProtect, VirtualQuery,
};

use crate::config;
use crate::log::log;

struct Patch {
    name: &'static str,
    address: usize,
    original: &'static [u8],
    replacement: &'static [u8],
}

const PATCHES: &[Patch] = &[Patch {
    // The resolution parser only keeps six 4:3 sizes; jump straight to its 32-bit check.
    name: "any resolution",
    address: 0x415db3,
    original: &[0x3d, 0x80, 0x02, 0x00, 0x00],
    replacement: &[0xeb, 0x64, 0x90, 0x90, 0x90],
}];

/// GameSettingsMgr::FindAvailableDisplayMode, which validates the video settings before every use.
const FIND_AVAILABLE_DISPLAY_MODE: usize = 0x414490;
const FIND_AVAILABLE_DISPLAY_MODE_START: &[u8] = &[0x56, 0x8b, 0x74, 0x24, 0x08, 0x57, 0x56, 0x8b, 0xf9];

/// The loading screen's state; its byte at +8 is set once the level is loaded.
const LOADING_SCREEN: usize = 0x93de28;
const LOADING_FINISHED: usize = 8;

static DONE: AtomicBool = AtomicBool::new(false);
static ORIGINAL_FIND_MODE: AtomicUsize = AtomicUsize::new(0);

/// Installs everything once the game is unpacked; earlier calls (or another game version) change nothing.
pub fn on_startup() {
    if DONE.load(Ordering::Relaxed) || !unsafe { matches(FIND_AVAILABLE_DISPLAY_MODE, FIND_AVAILABLE_DISPLAY_MODE_START) } {
        return;
    }
    DONE.store(true, Ordering::Relaxed);
    for patch in PATCHES {
        if unsafe { matches(patch.address, patch.original) } {
            unsafe { write(patch.address, patch.replacement) };
            log!("patched: {}", patch.name);
        } else {
            log!("not patched, unknown code: {}", patch.name);
        }
    }
    let hooked = unsafe {
        MinHook::create_hook(FIND_AVAILABLE_DISPLAY_MODE as *mut c_void, find_available_display_mode as *mut c_void)
            .and_then(|original| {
                ORIGINAL_FIND_MODE.store(original as usize, Ordering::Relaxed);
                MinHook::enable_hook(FIND_AVAILABLE_DISPLAY_MODE as *mut c_void)
            })
    };
    match hooked {
        Ok(()) => log!("hooked FindAvailableDisplayMode"),
        Err(e) => log!("cannot hook FindAvailableDisplayMode: {e:?}"),
    }
}

/// The game's video settings start with width, height and the fullscreen flag.
unsafe extern "thiscall" fn find_available_display_mode(manager: *mut c_void, settings: *mut u32) {
    let config = config::get();
    unsafe {
        *settings = config.width;
        *settings.add(1) = config.height;
        *settings.add(2) = u32::from(config.fullscreen);
        let original: unsafe extern "thiscall" fn(*mut c_void, *mut u32) =
            std::mem::transmute(ORIGINAL_FIND_MODE.load(Ordering::Relaxed));
        original(manager, settings);
        log!("display mode: {}x{} fullscreen={}", *settings, *settings.add(1), *settings.add(2));
    }
}

unsafe fn matches(address: usize, expected: &[u8]) -> bool {
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let queried = unsafe { VirtualQuery(address as *const c_void, &mut info, size_of::<MEMORY_BASIC_INFORMATION>()) };
    let readable = queried != 0
        && info.State == MEM_COMMIT
        && info.Protect & (PAGE_NOACCESS | PAGE_GUARD) == 0
        && address + expected.len() <= info.BaseAddress as usize + info.RegionSize;
    readable && unsafe { std::slice::from_raw_parts(address as *const u8, expected.len()) } == expected
}

unsafe fn write(address: usize, bytes: &[u8]) {
    let mut protection = 0;
    unsafe {
        VirtualProtect(address as *const c_void, bytes.len(), PAGE_EXECUTE_READWRITE, &mut protection);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len());
        VirtualProtect(address as *const c_void, bytes.len(), protection, &mut protection);
    }
}

/// Whether a loading screen is up and the game has finished loading behind it.
pub fn loading_finished() -> bool {
    if !DONE.load(Ordering::Relaxed) {
        return false;
    }
    let screen = unsafe { *(LOADING_SCREEN as *const usize) };
    screen != 0 && unsafe { *((screen + LOADING_FINISHED) as *const u8) } != 0
}
