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

/// The camera's projection setup ends in `push 0xd705f0; mov ecx, renderer; call SetViewProjection`.
const SET_PROJECTION_CALL: usize = 0x7452f4;
const SET_PROJECTION_CALL_ORIGINAL: &[u8] = &[0xe8, 0x27, 0x88, 0x00, 0x00];
const SET_VIEW_PROJECTION: usize = 0x74db20;
/// The renderer's view matrix, which SetViewProjection multiplies with the projection.
const RENDERER_VIEW: usize = 0x1b;
/// GameSettingsMgr, whose video settings at +0xc0 start with the render width and height.
const SETTINGS_MANAGER: usize = 0x93df34;
const RENDER_WIDTH: usize = 0xc0;
const RENDER_HEIGHT: usize = 0xc4;
/// The aspect ratio the game was made for; the engine keeps its horizontal view on wider screens.
const DESIGN_ASPECT: f32 = 4.0 / 3.0;

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
    log!("game unpacked");
    for patch in PATCHES {
        if unsafe { matches(patch.address, patch.original) } {
            unsafe { write(patch.address, patch.replacement) };
            log!("patched: {}", patch.name);
        } else {
            log!("not patched, unknown code: {}", patch.name);
        }
    }
    if config::get().widescreen {
        if unsafe { matches(SET_PROJECTION_CALL, SET_PROJECTION_CALL_ORIGINAL) } {
            let target = set_projection as *const () as isize - (SET_PROJECTION_CALL as isize + 5);
            let mut call = [0xe8u8, 0, 0, 0, 0];
            call[1..].copy_from_slice(&(target as i32).to_le_bytes());
            unsafe { write(SET_PROJECTION_CALL, &call) };
            log!("patched: widescreen projection");
        } else {
            log!("not patched, unknown code: widescreen projection");
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

/// The game's video settings start with width, height and the fullscreen flag; popww.ini may override them.
unsafe extern "thiscall" fn find_available_display_mode(manager: *mut c_void, settings: *mut u32) {
    let config = config::get();
    unsafe {
        if let Some((width, height)) = config.resolution {
            *settings = width;
            *settings.add(1) = height;
        }
        if let Some(fullscreen) = config.fullscreen {
            *settings.add(2) = u32::from(fullscreen);
        }
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

/// Hor+ widescreen: the engine keeps the horizontal field of view and crops the top and bottom on
/// wider screens (HUD included); this restores the 4:3 vertical view and widens the horizontal one.
unsafe extern "thiscall" fn set_projection(renderer: *mut c_void, matrix: *mut f32) {
    unsafe {
        let settings = *(SETTINGS_MANAGER as *const usize);
        let width = *((settings + RENDER_WIDTH) as *const i32) as f32;
        let height = *((settings + RENDER_HEIGHT) as *const i32) as f32;
        let (p00, p11) = (*matrix, *matrix.add(5));
        // Other ratios belong to render targets such as reflections.
        let view = std::slice::from_raw_parts((renderer as *const f32).add(RENDERER_VIEW), 16);
        let screen = width > 0.0 && height > 0.0 && p11 != 0.0 && ((p00 / p11).abs() - height / width).abs() < 0.01;
        // Cameras at the origin show the HUD, which the d3d hooks narrow to 4:3 themselves.
        let at_origin = view[12..15].iter().all(|&t| t == 0.0);
        if screen && !at_origin && width / height > DESIGN_ASPECT {
            let scale = DESIGN_ASPECT * height / width;
            *matrix *= scale;
            *matrix.add(5) *= scale;
        }
        let original: unsafe extern "thiscall" fn(*mut c_void, *mut f32) = std::mem::transmute(SET_VIEW_PROJECTION);
        original(renderer, matrix);
    }
}
