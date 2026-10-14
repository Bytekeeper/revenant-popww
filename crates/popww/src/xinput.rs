//! The first connected XInput gamepad; the same layout on every Windows and Wine version.

use std::sync::atomic::{AtomicUsize, Ordering};

use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::core::w;

pub const START: u16 = 0x0010;
pub const A: u16 = 0x1000;
pub const B: u16 = 0x2000;

/// XINPUT_GAMEPAD; triggers and sticks follow the buttons.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Gamepad {
    pub buttons: u16,
    rest: [u8; 10],
}

#[repr(C)]
#[derive(Default)]
struct State {
    packet: u32,
    gamepad: Gamepad,
}

type GetState = unsafe extern "system" fn(u32, *mut State) -> u32;

pub fn first_pad() -> Option<Gamepad> {
    static GET_STATE: AtomicUsize = AtomicUsize::new(usize::MAX);
    let mut get_state = GET_STATE.load(Ordering::Relaxed);
    if get_state == usize::MAX {
        let module = unsafe { LoadLibraryW(w!("xinput1_3.dll")) };
        get_state = if module.is_null() {
            0
        } else {
            unsafe { GetProcAddress(module, c"XInputGetState".as_ptr().cast()) }.map_or(0, |f| f as usize)
        };
        GET_STATE.store(get_state, Ordering::Relaxed);
    }
    if get_state == 0 {
        return None;
    }
    let get_state: GetState = unsafe { std::mem::transmute(get_state) };
    (0..4).find_map(|pad| {
        let mut state = State::default();
        (unsafe { get_state(pad, &mut state) } == 0).then_some(state.gamepad)
    })
}
