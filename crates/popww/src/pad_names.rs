//! The game names gamepad controls by their DirectInput numbers ("Btn 3", "Z Axis(-)"); with an
//! Xbox-style pad connected, prompts and the controls menu show its labels instead.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use minhook::MinHook;

use crate::game::matches;
use crate::log::log;
use crate::xinput;

/// A method returning "[name]" for an input (device, code), from the short or the tutorial's long names.
const CONTROL_NAME: usize = 0x4184a0;
const CONTROL_NAME_START: &[u8] = &[0x6a, 0xff, 0x68, 0x26, 0x46, 0x8b, 0x00];
/// The game's 16-byte result buffer, whose contents callers copy.
const NAME_BUFFER: usize = 0x93e968;
const NAME_BUFFER_SIZE: usize = 16;
const GAMEPAD: u32 = 2;

type ControlName = unsafe extern "thiscall" fn(*mut c_void, u32, u32, u32) -> *mut u8;

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

pub fn install() {
    if !unsafe { matches(CONTROL_NAME, CONTROL_NAME_START) } {
        log!("not patched, unknown code: Xbox button names");
        return;
    }
    let hooked = unsafe {
        MinHook::create_hook(CONTROL_NAME as *mut c_void, control_name as *mut c_void).and_then(|original| {
            ORIGINAL.store(original as usize, Ordering::Relaxed);
            MinHook::enable_hook(CONTROL_NAME as *mut c_void)
        })
    };
    match hooked {
        Ok(()) => log!("hooked control names"),
        Err(e) => log!("cannot hook control names: {e:?}"),
    }
}

unsafe extern "thiscall" fn control_name(this: *mut c_void, device: u32, code: u32, short: u32) -> *mut u8 {
    let original: ControlName = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Relaxed)) };
    let name = unsafe { original(this, device, code, short) };
    if device & 0xff != GAMEPAD || name as usize != NAME_BUFFER || !xbox_pad_connected() {
        return name;
    }
    if let Some(label) = xbox_label(code & 0xff) {
        let mut text = format!("[{label}]").into_bytes();
        text.truncate(NAME_BUFFER_SIZE - 1);
        text.push(0);
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), name, text.len()) };
    }
    name
}

/// DirectInput's numbering of an XInput pad: buttons, then two codes per axis (negative, positive).
fn xbox_label(code: u32) -> Option<&'static str> {
    const LABELS: [&str; 52] = [
        "A", "B", "X", "Y", "LB", "RB", "Back", "Start", "LS", "RS", "", "", "", "", "", "", "", "", "", "", "", "",
        "", "", "", "", "", "", "", "", "", "", "LS Left", "LS Right", "LS Up", "LS Down", "RT", "LT", "RS Left",
        "RS Right", "RS Up", "RS Down", "", "", "", "", "", "", "D-Pad Up", "D-Pad Down", "D-Pad Left", "D-Pad Right",
    ];
    LABELS.get(code as usize).copied().filter(|label| !label.is_empty())
}

/// Checked at most once a second; prompts ask every frame.
fn xbox_pad_connected() -> bool {
    static CHECKED: AtomicU64 = AtomicU64::new(0);
    let now = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() };
    let checked = CHECKED.load(Ordering::Relaxed);
    if now.saturating_sub(checked >> 1) >= 1000 {
        let connected = xinput::first_pad().is_some();
        CHECKED.store(now << 1 | u64::from(connected), Ordering::Relaxed);
        return connected;
    }
    checked & 1 != 0
}
