//! Answers for `Hardware.INI`, which the original launcher's hardware detection writes.
//!
//! That detection cannot recognise any hardware made after 2004 (and finds nothing under Wine), and
//! without the file the game has no display modes and quits. We describe the current machine instead.

use std::sync::OnceLock;

use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::core::w;

use crate::log::log;

pub const FILE_NAME: &str = "hardware.ini";

/// Formats every Direct3D 9 GPU of the last 15 years has; `D3DFMT_P8` (palettes) and `D3DFMT_D32` are gone from them.
const TEXTURE_FORMATS: &str =
    "D3DFMT_A8R8G8B8,D3DFMT_X8R8G8B8,D3DFMT_R5G6B5,D3DFMT_A1R5G5B5,D3DFMT_X1R5G5B5,D3DFMT_DXT1,D3DFMT_DXT3,D3DFMT_DXT5";
const RENDER_TARGETS: &str = "D3DFMT_A8R8G8B8,D3DFMT_R5G6B5";
const BUFFER_DEPTHS: &str = "D3DFMT_D16,D3DFMT_D24X8,D3DFMT_D24S8";

/// Capabilities of any current GPU are on; the flags that work around specific 2004 cards are off.
const CAPS: &[(&str, &str)] = &[
    ("NonPow2Textures", "1"),
    ("StreamsCount", "16"),
    ("PureDevice", "0"),
    ("WFog", "1"),
    ("ZFog", "1"),
    // Pixel (table) fog turns the whole menu scene white, at least under Wine; vertex fog works.
    ("PixelFog", "0"),
    ("FogRange", "1"),
    ("VertexFog", "1"),
    ("ZBias", "1"),
    ("SquareOnlyTextures", "0"),
    ("CanStretchRect", "1"),
    ("AlphaPalette", "0"),
    ("FullscreenGamma", "1"),
    ("AllowDisableVSync", "1"),
    ("ColorDiffusion", "0"),
    ("ForceVSFog", "0"),
    ("InvertFogRange", "0"),
    ("PixelShaders11", "1"),
    ("VertexShaders11", "1"),
    ("FFPHasTempReg", "1"),
    ("HeatShimmering", "1"),
    ("HardwareTnL", "1"),
    ("ShadowScaling", "0"),
    ("ZEnableFix", "0"),
    ("ShadowsEnabled", "1"),
    ("BlurFX", "1"),
    ("EnableAE", "1"),
];

/// Our value for a `Hardware.INI` key; None leaves the game's own default.
pub fn value(section: &str, key: &str) -> Option<String> {
    let section = section.to_ascii_uppercase();
    if let Some(value) = crate::config::hardware_override(&section, key) {
        return Some(value);
    }
    let fixed = match (section.as_str(), key) {
        ("VIDEO", "Resolutions") => return Some(resolutions().clone()),
        ("VIDEO", "AdapterFormats") => "D3DFMT_X8R8G8B8",
        ("D3DFMT_X8R8G8B8", "TextureFormats") => TEXTURE_FORMATS,
        ("D3DFMT_X8R8G8B8", "RenderTargets") => RENDER_TARGETS,
        ("D3DFMT_X8R8G8B8", "BufferDepths") => BUFFER_DEPTHS,
        ("SOUND", "Available") => "1",
        ("SOUND", "Legacy") => "0",
        ("CAPS", key) => CAPS.iter().find(|(name, _)| name.eq_ignore_ascii_case(key))?.1,
        _ => return None,
    };
    Some(fixed.to_owned())
}

/// The adapter's 32-bit modes as "width,height,32,…"; the game keeps the 4:3 ones it knows.
fn resolutions() -> &'static String {
    static LIST: OnceLock<String> = OnceLock::new();
    LIST.get_or_init(|| {
        let modes = unsafe { display_modes() }.unwrap_or_else(|e| {
            log!("cannot list display modes: {e}");
            Vec::new()
        });
        let list = modes.iter().map(|(w, h)| format!("{w},{h},32")).collect::<Vec<_>>().join(",");
        log!("resolutions: {list}");
        list
    })
}

const D3D_SDK_VERSION: u32 = 32;
const D3DFMT_X8R8G8B8: u32 = 22;
const RELEASE: usize = 2;
const GET_ADAPTER_MODE_COUNT: usize = 6;
const ENUM_ADAPTER_MODES: usize = 7;

#[repr(C)]
#[derive(Default)]
#[allow(dead_code)]
struct DisplayMode {
    width: u32,
    height: u32,
    refresh_rate: u32,
    format: u32,
}

type Direct3DCreate9 = unsafe extern "system" fn(u32) -> *mut *const usize;
type Release = unsafe extern "system" fn(*mut *const usize) -> u32;
type GetAdapterModeCount = unsafe extern "system" fn(*mut *const usize, u32, u32) -> u32;
type EnumAdapterModes = unsafe extern "system" fn(*mut *const usize, u32, u32, u32, *mut DisplayMode) -> i32;

/// Distinct resolutions of the primary adapter, through IDirect3D9's vtable.
unsafe fn display_modes() -> Result<Vec<(u32, u32)>, String> {
    let module = unsafe { LoadLibraryW(w!("d3d9.dll")) };
    if module.is_null() {
        return Err("d3d9.dll not loadable".into());
    }
    let create = unsafe { GetProcAddress(module, c"Direct3DCreate9".as_ptr().cast()) }.ok_or("no Direct3DCreate9")?;
    let create: Direct3DCreate9 = unsafe { std::mem::transmute(create) };
    let d3d = unsafe { create(D3D_SDK_VERSION) };
    if d3d.is_null() {
        return Err("Direct3DCreate9 failed".into());
    }
    let vtable = unsafe { *d3d };
    let slot = |i: usize| unsafe { *vtable.add(i) };
    let count: GetAdapterModeCount = unsafe { std::mem::transmute(slot(GET_ADAPTER_MODE_COUNT)) };
    let enumerate: EnumAdapterModes = unsafe { std::mem::transmute(slot(ENUM_ADAPTER_MODES)) };
    let release: Release = unsafe { std::mem::transmute(slot(RELEASE)) };

    let mut modes = Vec::new();
    for index in 0..unsafe { count(d3d, 0, D3DFMT_X8R8G8B8) } {
        let mut mode = DisplayMode::default();
        if unsafe { enumerate(d3d, 0, D3DFMT_X8R8G8B8, index, &mut mode) } >= 0
            && !modes.contains(&(mode.width, mode.height))
        {
            modes.push((mode.width, mode.height));
        }
    }
    unsafe { release(d3d) };
    Ok(modes)
}
