//! `popww.ini` next to the game: the settings the original launcher's dialog used to make.

use std::path::PathBuf;
use std::sync::OnceLock;

use windows_sys::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};
use windows_sys::Win32::System::WindowsProgramming::{GetPrivateProfileIntW, GetPrivateProfileStringW};

use crate::log::log;

const DEFAULT_INI: &str = "\
; popww settings for Prince of Persia: Warrior Within.

[Display]
; Resolution; 0 uses the desktop resolution.
Width=0
Height=0
; 1 = fullscreen, 0 = window.
Fullscreen=1
; 1 = the 3D view widens with the screen (Hor+), 0 = the original 4:3 view stretched.
Widescreen=1

[Video]
; 1 leaves out the logo videos at startup. Cinematics end with Esc, Space, Enter or gamepad A/B/Start.
SkipLogos=1

; Overrides for the hardware description the old launcher wrote to Hardware.INI,
; as [Hardware.<section>] key=value, e.g.:
; [Hardware.CAPS]
; InvertFogRange=1
";

/// A `[Hardware.<section>]` value from `popww.ini`.
pub fn hardware_override(section: &str, key: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let file: Vec<u16> = crate::module_dir().join("popww.ini").as_os_str().encode_wide().chain([0]).collect();
    let mut out = [0u16; 512];
    let len = unsafe {
        GetPrivateProfileStringW(
            wide(&format!("Hardware.{section}")).as_ptr(),
            wide(key).as_ptr(),
            wide("\u{1}").as_ptr(),
            out.as_mut_ptr(),
            out.len() as u32,
            file.as_ptr(),
        )
    } as usize;
    let value = String::from_utf16_lossy(&out[..len]);
    (value != "\u{1}").then_some(value)
}

pub struct Config {
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub widescreen: bool,
    pub skip_logos: bool,
}

pub fn get() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(load)
}

fn load() -> Config {
    let path = crate::module_dir().join("popww.ini");
    if !path.exists() && std::fs::write(&path, DEFAULT_INI.replace('\n', "\r\n")).is_ok() {
        log!("wrote default {}", path.display());
    }
    let read = |section: &str, key: &str, default: i32| read_int(&path, section, key, default);
    let (desktop_width, desktop_height) = desktop_resolution();
    let config = Config {
        width: positive_or(read("Display", "Width", 0), desktop_width),
        height: positive_or(read("Display", "Height", 0), desktop_height),
        fullscreen: read("Display", "Fullscreen", 1) != 0,
        widescreen: read("Display", "Widescreen", 1) != 0,
        skip_logos: read("Video", "SkipLogos", 1) != 0,
    };
    log!(
        "config: {}x{} fullscreen={} widescreen={} skip_logos={}",
        config.width,
        config.height,
        config.fullscreen,
        config.widescreen,
        config.skip_logos
    );
    config
}

fn positive_or(value: i32, fallback: u32) -> u32 {
    u32::try_from(value).ok().filter(|&v| v > 0).unwrap_or(fallback)
}

fn read_int(path: &PathBuf, section: &str, key: &str, default: i32) -> i32 {
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    use std::os::windows::ffi::OsStrExt;
    let file: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    unsafe { GetPrivateProfileIntW(wide(section).as_ptr(), wide(key).as_ptr(), default, file.as_ptr()) as i32 }
}

/// The display mode in real pixels; GetSystemMetrics would report DPI-scaled sizes to this unaware game.
fn desktop_resolution() -> (u32, u32) {
    let mut mode: DEVMODEW = unsafe { std::mem::zeroed() };
    mode.dmSize = size_of::<DEVMODEW>() as u16;
    if unsafe { EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut mode) } == 0 {
        log!("desktop resolution unknown, using 1024x768");
        return (1024, 768);
    }
    (mode.dmPelsWidth, mode.dmPelsHeight)
}
