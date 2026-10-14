//! Inline hooks on the kernel32 ANSI file functions the game and its DLLs call with lowercased paths.

use std::cell::Cell;
use std::ffi::{CStr, CString, OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use minhook::MinHook;
use windows_sys::Win32::Foundation::{BOOL, HANDLE};
use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar, WideCharToMultiByte};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::WIN32_FIND_DATAA;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::core::{PCSTR, w};

use crate::{casefix, config, game, gamepads, hardware};
use crate::log::log;

thread_local! {
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

/// The game's path with its on-disk spelling, if that differs.
unsafe fn fix(path: PCSTR) -> Option<CString> {
    if path.is_null() || IN_HOOK.get() {
        return None;
    }
    IN_HOOK.set(true);
    let original = unsafe { CStr::from_ptr(path.cast()) };
    let fixed = to_wide(original.to_bytes())
        .and_then(|wide| {
            let path = PathBuf::from(OsString::from_wide(&wide));
            let real = casefix::resolve(&path);
            if gamepads::is_table(&path) {
                return gamepads::extended(real.as_deref().unwrap_or(&path)).map(PathBuf::from);
            }
            real
        })
        .and_then(|real| to_ansi(&real.as_os_str().encode_wide().collect::<Vec<_>>()));
    if let Some(fixed) = &fixed {
        log!("path {} -> {}", original.to_string_lossy(), fixed.to_string_lossy());
    }
    IN_HOOK.set(false);
    fixed
}

fn to_wide(ansi: &[u8]) -> Option<Vec<u16>> {
    if ansi.is_empty() {
        return None;
    }
    let len = unsafe { MultiByteToWideChar(CP_ACP, 0, ansi.as_ptr(), ansi.len() as i32, std::ptr::null_mut(), 0) };
    let mut wide = vec![0u16; usize::try_from(len).ok().filter(|&n| n > 0)?];
    unsafe { MultiByteToWideChar(CP_ACP, 0, ansi.as_ptr(), ansi.len() as i32, wide.as_mut_ptr(), len) };
    Some(wide)
}

/// None if the path has characters the ANSI code page cannot express; the game then keeps its own.
fn to_ansi(wide: &[u16]) -> Option<CString> {
    let mut lossy: BOOL = 0;
    let len = unsafe {
        WideCharToMultiByte(CP_ACP, 0, wide.as_ptr(), wide.len() as i32, std::ptr::null_mut(), 0, std::ptr::null(), &mut lossy)
    };
    let mut ansi = vec![0u8; usize::try_from(len).ok().filter(|&n| n > 0)?];
    unsafe {
        WideCharToMultiByte(CP_ACP, 0, wide.as_ptr(), wide.len() as i32, ansi.as_mut_ptr(), len, std::ptr::null(), &mut lossy)
    };
    if lossy != 0 {
        return None;
    }
    CString::new(ansi).ok()
}

fn or_original(fixed: &Option<CString>, original: PCSTR) -> PCSTR {
    fixed.as_ref().map_or(original, |c| c.as_ptr().cast())
}

/// Declares a hooked function: its trampoline slot, the original's signature and a detour that
/// fixes the listed path arguments.
macro_rules! hook {
    ($name:ident, $slot:ident, fn($($arg:ident: $ty:ty),*) -> $ret:ty, paths($($path:ident),+)) => {
        static $slot: AtomicUsize = AtomicUsize::new(0);
        #[allow(non_snake_case)]
        unsafe extern "system" fn $name($($arg: $ty),*) -> $ret {
            type Original = unsafe extern "system" fn($($ty),*) -> $ret;
            let original: Original = unsafe { std::mem::transmute($slot.load(Ordering::Relaxed)) };
            $(let fixed = unsafe { fix($path) }; let $path = or_original(&fixed, $path);)+
            unsafe { original($($arg),*) }
        }
    };
}

hook!(CreateFileA, CREATE_FILE_A,
    fn(name: PCSTR, access: u32, share: u32, security: *const SECURITY_ATTRIBUTES, disposition: u32, flags: u32, template: HANDLE) -> HANDLE,
    paths(name));
hook!(FindFirstFileA, FIND_FIRST_FILE_A, fn(name: PCSTR, data: *mut WIN32_FIND_DATAA) -> HANDLE, paths(name));
hook!(GetFileAttributesA, GET_FILE_ATTRIBUTES_A, fn(name: PCSTR) -> u32, paths(name));
hook!(SetFileAttributesA, SET_FILE_ATTRIBUTES_A, fn(name: PCSTR, attributes: u32) -> BOOL, paths(name));
hook!(DeleteFileA, DELETE_FILE_A, fn(name: PCSTR) -> BOOL, paths(name));
hook!(CreateDirectoryA, CREATE_DIRECTORY_A, fn(name: PCSTR, security: *const SECURITY_ATTRIBUTES) -> BOOL, paths(name));
hook!(RemoveDirectoryA, REMOVE_DIRECTORY_A, fn(name: PCSTR) -> BOOL, paths(name));
hook!(MoveFileA, MOVE_FILE_A, fn(from: PCSTR, to: PCSTR) -> BOOL, paths(from, to));
hook!(SetCurrentDirectoryA, SET_CURRENT_DIRECTORY_A, fn(name: PCSTR) -> BOOL, paths(name));

static GET_PRIVATE_PROFILE_INT_A: AtomicUsize = AtomicUsize::new(0);
static GET_PRIVATE_PROFILE_STRING_A: AtomicUsize = AtomicUsize::new(0);

#[allow(non_snake_case)]
unsafe extern "system" fn GetPrivateProfileIntA(section: PCSTR, key: PCSTR, default: i32, file: PCSTR) -> u32 {
    if let Some(value) = unsafe { hardware_value(section, key, file) } {
        return value.parse().unwrap_or(default as u32);
    }
    type Original = unsafe extern "system" fn(PCSTR, PCSTR, i32, PCSTR) -> u32;
    let original: Original = unsafe { std::mem::transmute(GET_PRIVATE_PROFILE_INT_A.load(Ordering::Relaxed)) };
    let fixed = unsafe { fix(file) };
    unsafe { original(section, key, default, or_original(&fixed, file)) }
}

#[allow(non_snake_case)]
unsafe extern "system" fn GetPrivateProfileStringA(
    section: PCSTR,
    key: PCSTR,
    default: PCSTR,
    out: *mut u8,
    size: u32,
    file: PCSTR,
) -> u32 {
    if let Some(value) = unsafe { hardware_value(section, key, file) }
        && !out.is_null()
        && size > 0
    {
        let len = value.len().min(size as usize - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(value.as_ptr(), out, len);
            *out.add(len) = 0;
        }
        return len as u32;
    }
    type Original = unsafe extern "system" fn(PCSTR, PCSTR, PCSTR, *mut u8, u32, PCSTR) -> u32;
    let original: Original = unsafe { std::mem::transmute(GET_PRIVATE_PROFILE_STRING_A.load(Ordering::Relaxed)) };
    let fixed = unsafe { fix(file) };
    unsafe { original(section, key, default, out, size, or_original(&fixed, file)) }
}

/// Our answer if the game reads a key of `Hardware.INI`.
unsafe fn hardware_value(section: PCSTR, key: PCSTR, file: PCSTR) -> Option<String> {
    if section.is_null() || key.is_null() || file.is_null() {
        return None;
    }
    let file = unsafe { CStr::from_ptr(file.cast()) }.to_bytes();
    let name = file.rsplit(|&b| b == b'\\' || b == b'/').next()?;
    if !name.eq_ignore_ascii_case(hardware::FILE_NAME.as_bytes()) {
        return None;
    }
    let section = unsafe { CStr::from_ptr(section.cast()) }.to_string_lossy();
    let key = unsafe { CStr::from_ptr(key.cast()) }.to_string_lossy();
    let value = hardware::value(&section, &key);
    log!("Hardware.INI [{section}] {key} = {}", value.as_deref().unwrap_or("(game default)"));
    value
}
hook!(WritePrivateProfileStringA, WRITE_PRIVATE_PROFILE_STRING_A,
    fn(section: PCSTR, key: PCSTR, value: PCSTR, file: PCSTR) -> BOOL, paths(file));

static GET_COMMAND_LINE_A: AtomicUsize = AtomicUsize::new(0);

/// The C runtime's first call after UPX has unpacked the game; the moment to patch it.
#[allow(non_snake_case)]
unsafe extern "system" fn GetCommandLineA() -> PCSTR {
    // Before the game switches the display mode, so "desktop resolution" is still the desktop's.
    config::get();
    game::on_startup();
    let original: unsafe extern "system" fn() -> PCSTR =
        unsafe { std::mem::transmute(GET_COMMAND_LINE_A.load(Ordering::Relaxed)) };
    unsafe { original() }
}

pub fn install_all() {
    let hooks: [(&CStr, *const (), &AtomicUsize); 13] = [
        (c"GetCommandLineA", GetCommandLineA as *const (), &GET_COMMAND_LINE_A),
        (c"CreateFileA", CreateFileA as *const (), &CREATE_FILE_A),
        (c"FindFirstFileA", FindFirstFileA as *const (), &FIND_FIRST_FILE_A),
        (c"GetFileAttributesA", GetFileAttributesA as *const (), &GET_FILE_ATTRIBUTES_A),
        (c"SetFileAttributesA", SetFileAttributesA as *const (), &SET_FILE_ATTRIBUTES_A),
        (c"DeleteFileA", DeleteFileA as *const (), &DELETE_FILE_A),
        (c"CreateDirectoryA", CreateDirectoryA as *const (), &CREATE_DIRECTORY_A),
        (c"RemoveDirectoryA", RemoveDirectoryA as *const (), &REMOVE_DIRECTORY_A),
        (c"MoveFileA", MoveFileA as *const (), &MOVE_FILE_A),
        (c"SetCurrentDirectoryA", SetCurrentDirectoryA as *const (), &SET_CURRENT_DIRECTORY_A),
        (c"GetPrivateProfileIntA", GetPrivateProfileIntA as *const (), &GET_PRIVATE_PROFILE_INT_A),
        (c"GetPrivateProfileStringA", GetPrivateProfileStringA as *const (), &GET_PRIVATE_PROFILE_STRING_A),
        (c"WritePrivateProfileStringA", WritePrivateProfileStringA as *const (), &WRITE_PRIVATE_PROFILE_STRING_A),
    ];
    let kernel32 = unsafe { GetModuleHandleW(w!("kernel32.dll")) };
    for (name, detour, slot) in hooks {
        match unsafe { install(kernel32, name, detour, slot) } {
            Ok(()) => log!("hooked {}", name.to_string_lossy()),
            Err(e) => log!("cannot hook {}: {e}", name.to_string_lossy()),
        }
    }
}

unsafe fn install(module: *mut c_void, name: &CStr, detour: *const (), slot: &AtomicUsize) -> Result<(), String> {
    let target = unsafe { GetProcAddress(module, name.as_ptr().cast()) }.ok_or("not exported")? as *mut c_void;
    let original = unsafe { MinHook::create_hook(target, detour as *mut c_void) }.map_err(|e| format!("{e:?}"))?;
    slot.store(original as usize, Ordering::Relaxed);
    unsafe { MinHook::enable_hook(target) }.map_err(|e| format!("{e:?}"))
}
