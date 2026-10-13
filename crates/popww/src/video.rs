//! Skippable videos: cinematics and logos end on a key or gamepad press, logos can be left out.

use std::ffi::{CStr, c_void};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use minhook::MinHook;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_RETURN, VK_SPACE};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows_sys::core::w;

use crate::config;
use crate::log::log;

const LOGOS: &[&str] = &["ubisoft.bik", "intel.bik", "ubimtl.bik", "logorp.bik"];
/// BinkOpen's first argument is a file handle instead of a name.
const BINK_FILE_HANDLE: u32 = 0x0080_0000;
const FRAMES: usize = 0x08;

static ORIGINAL_OPEN: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_WAIT: AtomicUsize = AtomicUsize::new(0);
static GOTO: AtomicUsize = AtomicUsize::new(0);
/// The playing video that may be skipped, and whether all skip buttons were released since it started.
static SKIPPABLE: AtomicUsize = AtomicUsize::new(0);
static RELEASED: AtomicBool = AtomicBool::new(false);

type BinkOpen = unsafe extern "system" fn(*const u8, u32) -> *mut c_void;
type BinkWait = unsafe extern "system" fn(*mut c_void) -> i32;
type BinkGoto = unsafe extern "system" fn(*mut c_void, u32, i32);

pub fn install() {
    let bink = unsafe { GetModuleHandleW(w!("binkw32.dll")) };
    if bink.is_null() {
        log!("binkw32.dll not loaded, videos stay unskippable");
        return;
    }
    let export = |name: &CStr| unsafe { GetProcAddress(bink, name.as_ptr().cast()) }.map(|f| f as *mut c_void);
    let (Some(open), Some(wait), Some(goto)) = (export(c"_BinkOpen@8"), export(c"_BinkWait@4"), export(c"_BinkGoto@12"))
    else {
        log!("binkw32.dll lacks BinkOpen/BinkWait/BinkGoto, videos stay unskippable");
        return;
    };
    GOTO.store(goto as usize, Ordering::Relaxed);
    for (target, detour, slot, name) in [
        (open, bink_open as *mut c_void, &ORIGINAL_OPEN, "BinkOpen"),
        (wait, bink_wait as *mut c_void, &ORIGINAL_WAIT, "BinkWait"),
    ] {
        let hooked = unsafe {
            MinHook::create_hook(target, detour).and_then(|original| {
                slot.store(original as usize, Ordering::Relaxed);
                MinHook::enable_hook(target)
            })
        };
        match hooked {
            Ok(()) => log!("hooked {name}"),
            Err(e) => log!("cannot hook {name}: {e:?}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Other = 0,
    Logo = 1,
    Cinematic = 2,
}

/// Kind of the video last opened by name: the game reopens it right away through its own file
/// handle (past a 30-byte header Bink cannot read), and that handle carries no name.
static LAST_KIND: AtomicUsize = AtomicUsize::new(Kind::Other as usize);

unsafe extern "system" fn bink_open(name: *const u8, flags: u32) -> *mut c_void {
    let kind = if flags & BINK_FILE_HANDLE != 0 || name.is_null() {
        match LAST_KIND.load(Ordering::Relaxed) {
            1 => Kind::Logo,
            2 => Kind::Cinematic,
            _ => Kind::Other,
        }
    } else {
        let file = unsafe { CStr::from_ptr(name.cast()) }.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
        let kind = if LOGOS.contains(&file.rsplit('\\').next().unwrap_or_default()) {
            Kind::Logo
        } else if file.contains("\\cine\\") {
            Kind::Cinematic
        } else {
            Kind::Other
        };
        LAST_KIND.store(kind as usize, Ordering::Relaxed);
        kind
    };
    if kind == Kind::Logo && config::get().skip_logos {
        return std::ptr::null_mut();
    }
    let original: BinkOpen = unsafe { std::mem::transmute(ORIGINAL_OPEN.load(Ordering::Relaxed)) };
    let video = unsafe { original(name, flags) };
    if !video.is_null() {
        let skippable = kind != Kind::Other;
        SKIPPABLE.store(if skippable { video as usize } else { 0 }, Ordering::Relaxed);
        RELEASED.store(false, Ordering::Relaxed);
    }
    video
}

unsafe extern "system" fn bink_wait(video: *mut c_void) -> i32 {
    if !video.is_null() && SKIPPABLE.load(Ordering::Relaxed) == video as usize {
        let pressed = skip_pressed();
        if !pressed {
            RELEASED.store(true, Ordering::Relaxed);
        } else if RELEASED.load(Ordering::Relaxed) {
            SKIPPABLE.store(0, Ordering::Relaxed);
            let goto: BinkGoto = unsafe { std::mem::transmute(GOTO.load(Ordering::Relaxed)) };
            // The game ends a video once its last decoded frame is the final one.
            unsafe { goto(video, *(video.cast::<u8>().add(FRAMES).cast::<u32>()), 0) };
            log!("skipped video");
        }
    }
    let original: BinkWait = unsafe { std::mem::transmute(ORIGINAL_WAIT.load(Ordering::Relaxed)) };
    unsafe { original(video) }
}

fn skip_pressed() -> bool {
    let mut owner = 0;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut owner) };
    if owner != unsafe { GetCurrentProcessId() } {
        return false;
    }
    // Bit 0 catches taps that fall between two video frames.
    let key = |vk: u16| unsafe { GetAsyncKeyState(i32::from(vk)) } as u16 & 0x8001 != 0;
    key(VK_ESCAPE) || key(VK_SPACE) || key(VK_RETURN) || gamepad_pressed()
}

const XINPUT_A: u16 = 0x1000;
const XINPUT_B: u16 = 0x2000;
const XINPUT_START: u16 = 0x0010;

#[repr(C)]
#[derive(Default)]
struct XInputState {
    packet: u32,
    buttons: u16,
    triggers: [u8; 2],
    thumbs: [i16; 4],
}

type XInputGetState = unsafe extern "system" fn(u32, *mut XInputState) -> u32;

fn gamepad_pressed() -> bool {
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
        return false;
    }
    let get_state: XInputGetState = unsafe { std::mem::transmute(get_state) };
    (0..4).any(|pad| {
        let mut state = XInputState::default();
        let connected = unsafe { get_state(pad, &mut state) } == 0;
        connected && state.buttons & (XINPUT_A | XINPUT_B | XINPUT_START) != 0
    })
}
