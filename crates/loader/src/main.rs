//! Replacement for the game's launcher: starts `pop2.exe` with `popww.dll` loaded.

#![windows_subsystem = "windows"]

mod inject;

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
use windows_sys::Win32::System::Environment::GetCommandLineW;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateMutexW, CreateProcessW, GetExitCodeProcess, INFINITE, PROCESS_INFORMATION, ResumeThread,
    STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
use windows_sys::core::w;

const GAME: &str = "pop2.exe";
const DLL: &str = "popww.dll";

fn main() {
    match run() {
        Ok(code) => std::process::exit(code as i32),
        Err(e) => {
            let text = wide(&format!("Prince of Persia could not be started:\n\n{e}"));
            unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), w!("Prince of Persia"), MB_ICONERROR) };
            std::process::exit(1);
        }
    }
}

fn run() -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| format!("own path unknown: {e}"))?;
    let dir = exe.parent().ok_or("own directory unknown")?;
    let game = dir.join(GAME);
    for file in [GAME, DLL] {
        if !dir.join(file).is_file() {
            return Err(format!("{file} is missing in {}", dir.display()));
        }
    }

    // pop2.exe only starts while the original launcher's mutex exists.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), FALSE, w!("POP5Launcher")) };
    if mutex.is_null() {
        return Err(format!("cannot create the launcher mutex: {}", std::io::Error::last_os_error()));
    }

    let process = start_suspended(&game, dir)?;
    if let Err(e) = unsafe { inject::add_import(process.hProcess, &game, DLL, "popww_init") } {
        unsafe { TerminateProcess(process.hProcess, 1) };
        return Err(format!("cannot load {DLL} into the game: {e}"));
    }
    unsafe { ResumeThread(process.hThread) };

    // Waiting keeps the mutex alive and lets Heroic or Steam see when the game ends.
    let mut code = 0;
    unsafe {
        WaitForSingleObject(process.hProcess, INFINITE);
        GetExitCodeProcess(process.hProcess, &mut code);
        close(process.hThread);
        close(process.hProcess);
        close(mutex);
    }
    Ok(code)
}

fn start_suspended(game: &Path, dir: &Path) -> Result<PROCESS_INFORMATION, String> {
    let mut command: Vec<u16> = format!("\"{}\"", game.display()).encode_utf16().collect();
    command.extend(own_arguments());
    command.push(0);
    let game_w = wide_path(game);
    let dir_w = wide_path(dir);
    let startup = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..unsafe { std::mem::zeroed() } };
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            game_w.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            FALSE,
            CREATE_SUSPENDED,
            std::ptr::null(),
            dir_w.as_ptr(),
            &startup,
            &mut process,
        )
    };
    if ok == 0 {
        return Err(format!("cannot start {}: {}", game.display(), std::io::Error::last_os_error()));
    }
    Ok(process)
}

/// Our command line after our own program name, unparsed, so arguments like `-safe` reach the game unchanged.
fn own_arguments() -> Vec<u16> {
    let line = unsafe {
        let start = GetCommandLineW();
        let len = (0..).take_while(|&i| *start.add(i) != 0).count();
        std::slice::from_raw_parts(start, len)
    };
    let quote = u16::from(b'"');
    let end_of_name = if line.first() == Some(&quote) {
        line[1..].iter().position(|&c| c == quote).map_or(line.len(), |i| i + 2)
    } else {
        line.iter().position(|&c| c == u16::from(b' ') || c == u16::from(b'\t')).unwrap_or(line.len())
    };
    line[end_of_name..].to_vec()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain([0]).collect()
}

unsafe fn close(handle: HANDLE) {
    unsafe { CloseHandle(handle) };
}
