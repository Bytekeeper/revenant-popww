//! Adds an import of our DLL to a suspended process, so the system loader maps it before the game runs.
//!
//! This is the Detours technique: a new import descriptor table is written outside the image and
//! the in-memory PE header is pointed at it. The game file on disk stays untouched.

use std::ffi::c_void;
use std::path::Path;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualProtectEx};

const DESCRIPTOR: usize = 20;
const RELOCS_STRIPPED: u16 = 0x0001;
const PE32_MAGIC: u16 = 0x10b;
const IMPORT_DIR: usize = 1;
const BOUND_IMPORT_DIR: usize = 11;
const ALLOCATION_GRANULARITY: u32 = 0x10000;

struct Headers {
    image_base: u32,
    size_of_image: u32,
    data_directories: usize,
}

/// Reads the PE layout from the file on disk; without relocations the image loads at its preferred base.
fn headers(game: &Path) -> Result<(Headers, Vec<u8>), String> {
    let file = std::fs::read(game).map_err(|e| format!("cannot read {}: {e}", game.display()))?;
    let u16_at = |o: usize| file.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| file.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let nt = u32_at(0x3c).ok_or("not a PE file")? as usize;
    if file.get(nt..nt + 4) != Some(b"PE\0\0") {
        return Err("not a PE file".into());
    }
    let characteristics = u16_at(nt + 22).ok_or("truncated header")?;
    let optional = nt + 24;
    if u16_at(optional) != Some(PE32_MAGIC) {
        return Err("not a 32-bit executable".into());
    }
    if characteristics & RELOCS_STRIPPED == 0 {
        return Err("the executable is relocatable; unsupported game version".into());
    }
    let headers = Headers {
        image_base: u32_at(optional + 28).ok_or("truncated header")?,
        size_of_image: u32_at(optional + 56).ok_or("truncated header")?,
        data_directories: optional + 96,
    };
    let size_of_headers = u32_at(optional + 60).ok_or("truncated header")? as usize;
    Ok((headers, file[..size_of_headers.min(file.len())].to_vec()))
}

pub unsafe fn add_import(process: HANDLE, game: &Path, dll: &str, function: &str) -> Result<(), String> {
    let (h, file_header) = headers(game)?;
    let base = h.image_base as usize;
    let mut header = vec![0u8; file_header.len()];
    unsafe { read(process, base, &mut header)? };
    if header[..2] != *b"MZ" {
        return Err(format!("no image at {base:#x}"));
    }

    let directory = |i: usize| h.data_directories + 8 * i;
    let u32_at = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let old_rva = u32_at(&header, directory(IMPORT_DIR)) as usize;
    let old_size = u32_at(&header, directory(IMPORT_DIR) + 4) as usize;
    let mut old = vec![0u8; old_size];
    unsafe { read(process, base + old_rva, &mut old)? };
    let descriptors: Vec<&[u8]> = old.chunks_exact(DESCRIPTOR).take_while(|d| d.iter().any(|&b| b != 0)).collect();

    let block_addr = unsafe { allocate_after_image(process, &h, 0x1000)? };
    let rva = |offset: usize| (block_addr - base + offset) as u32;

    // Layout: descriptors (ours, the originals, terminator), lookup table, address table, hint/name, DLL name.
    let table_len = DESCRIPTOR * (descriptors.len() + 2);
    let lookup = table_len;
    let address = lookup + 8;
    let hint_name = address + 8;
    let dll_name = hint_name + 2 + function.len() + 2;
    let mut block = vec![0u8; dll_name + dll.len() + 1];

    let ours = [rva(lookup), 0, 0, rva(dll_name), rva(address)];
    for (i, field) in ours.iter().enumerate() {
        block[4 * i..4 * i + 4].copy_from_slice(&field.to_le_bytes());
    }
    for (i, descriptor) in descriptors.iter().enumerate() {
        block[DESCRIPTOR * (i + 1)..DESCRIPTOR * (i + 2)].copy_from_slice(descriptor);
    }
    block[lookup..lookup + 4].copy_from_slice(&rva(hint_name).to_le_bytes());
    block[address..address + 4].copy_from_slice(&rva(hint_name).to_le_bytes());
    block[hint_name + 2..hint_name + 2 + function.len()].copy_from_slice(function.as_bytes());
    block[dll_name..dll_name + dll.len()].copy_from_slice(dll.as_bytes());
    unsafe { write(process, block_addr, &block)? };

    let mut patched = header.clone();
    patched[directory(IMPORT_DIR)..directory(IMPORT_DIR) + 4].copy_from_slice(&rva(0).to_le_bytes());
    patched[directory(IMPORT_DIR) + 4..directory(IMPORT_DIR) + 8].copy_from_slice(&(table_len as u32).to_le_bytes());
    // A bound import table would let the loader skip our descriptor.
    patched[directory(BOUND_IMPORT_DIR)..directory(BOUND_IMPORT_DIR) + 8].fill(0);

    let mut protection = 0;
    if unsafe { VirtualProtectEx(process, base as *const c_void, header.len(), PAGE_READWRITE, &mut protection) } == 0 {
        return Err(format!("cannot unprotect the header: {}", std::io::Error::last_os_error()));
    }
    let written = unsafe { write(process, base, &patched) };
    unsafe { VirtualProtectEx(process, base as *const c_void, header.len(), protection, &mut protection) };
    written
}

/// Import RVAs are unsigned offsets from the image base, so the block must lie above the image.
unsafe fn allocate_after_image(process: HANDLE, h: &Headers, size: usize) -> Result<usize, String> {
    let start = (h.image_base + h.size_of_image).next_multiple_of(ALLOCATION_GRANULARITY);
    for addr in (start..0x7fff_0000).step_by(ALLOCATION_GRANULARITY as usize) {
        let block = unsafe {
            VirtualAllocEx(process, addr as *const c_void, size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE)
        };
        if !block.is_null() {
            return Ok(block as usize);
        }
    }
    Err("no free memory above the game image".into())
}

unsafe fn read(process: HANDLE, addr: usize, buf: &mut [u8]) -> Result<(), String> {
    let ok = unsafe {
        ReadProcessMemory(process, addr as *const c_void, buf.as_mut_ptr().cast(), buf.len(), std::ptr::null_mut())
    };
    if ok == 0 {
        return Err(format!("cannot read game memory at {addr:#x}: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

unsafe fn write(process: HANDLE, addr: usize, buf: &[u8]) -> Result<(), String> {
    let ok = unsafe {
        WriteProcessMemory(process, addr as *const c_void, buf.as_ptr().cast(), buf.len(), std::ptr::null_mut())
    };
    if ok == 0 {
        return Err(format!("cannot write game memory at {addr:#x}: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}
