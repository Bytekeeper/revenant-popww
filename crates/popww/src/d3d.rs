//! Direct3D 9 hooks that keep menus, HUD and videos at 4:3 on wider screens.
//!
//! The game draws its 2D with pre-transformed (`POSITIONT`) vertices in screen pixels, which skip
//! vertex processing. Those draws get a twin declaration with plain `POSITION` and a shader of ours
//! that maps the pixels to the 4:3 middle of the screen and passes everything else on unchanged.

use std::ffi::{CStr, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use minhook::MinHook;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetWindowLongW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER,
    SetWindowPos,
};
use windows_sys::core::w;

use crate::config;
use crate::log::log;

// IDirect3D9, IDirect3DDevice9 and resource vtable slots.
const CREATE_DEVICE: usize = 16;
const RESET: usize = 16;
const GET_RENDER_TARGET: usize = 38;
const GET_TEXTURE: usize = 64;
const DRAW_PRIMITIVE: usize = 81;
const DRAW_INDEXED_PRIMITIVE: usize = 82;
const CREATE_VERTEX_DECLARATION: usize = 86;
const SET_VERTEX_DECLARATION: usize = 87;
const SET_FVF: usize = 89;
const CREATE_VERTEX_SHADER: usize = 91;
const SET_VERTEX_SHADER: usize = 92;
pub(crate) const SET_VERTEX_SHADER_CONSTANT_F: usize = 94;
pub(crate) const GET_VERTEX_SHADER_CONSTANT_F: usize = 95;
pub(crate) const RELEASE: usize = 2;
const RESOURCE_GET_TYPE: usize = 10;
const SURFACE_GET_DESC: usize = 12;
const TEXTURE_GET_LEVEL_DESC: usize = 17;

const RESOURCE_TYPE_TEXTURE: u32 = 3;
const USAGE_RENDER_TARGET: u32 = 1;
pub(crate) const USAGE_POSITION: u8 = 0;
const USAGE_TEXCOORD: u8 = 5;
const USAGE_POSITIONT: u8 = 9;
const USAGE_COLOR: u8 = 10;
const DECLARATION_END_STREAM: u16 = 0xff;
const MAX_ELEMENTS: usize = 16;

static ORIGINAL_CREATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CREATE_DEVICE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_RESET: AtomicUsize = AtomicUsize::new(0);
/// The back buffer's width and height, packed; 0 until the device exists.
static SCREEN: AtomicU64 = AtomicU64::new(0);
static WINDOW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CREATE_DECLARATION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SET_DECLARATION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SET_FVF: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SET_SHADER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DRAW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DRAW_INDEXED: AtomicUsize = AtomicUsize::new(0);

pub(crate) type Interface = *mut *const usize;
type Direct3DCreate9 = unsafe extern "system" fn(u32) -> Interface;
type CreateDevice = unsafe extern "system" fn(Interface, u32, u32, *mut c_void, u32, *mut PresentParameters, *mut Interface) -> i32;
type Reset = unsafe extern "system" fn(Interface, *mut PresentParameters) -> i32;

/// The start of D3DPRESENT_PARAMETERS.
#[repr(C)]
struct PresentParameters {
    back_buffer_width: u32,
    back_buffer_height: u32,
    back_buffer_format: u32,
    back_buffer_count: u32,
    multi_sample_type: u32,
    multi_sample_quality: u32,
    swap_effect: u32,
    device_window: HWND,
    windowed: i32,
    enable_auto_depth_stencil: i32,
    auto_depth_stencil_format: u32,
    flags: u32,
    refresh_rate: u32,
    presentation_interval: u32,
}
type CreateShader = unsafe extern "system" fn(Interface, *const u32, *mut Interface) -> i32;
type CreateDeclaration = unsafe extern "system" fn(Interface, *const Element, *mut Interface) -> i32;
type SetObject = unsafe extern "system" fn(Interface, Interface) -> i32;
type Draw = unsafe extern "system" fn(Interface, u32, u32, u32) -> i32;
type DrawIndexed = unsafe extern "system" fn(Interface, u32, i32, u32, u32, u32, u32) -> i32;
pub(crate) type ShaderConstants = unsafe extern "system" fn(Interface, u32, *mut f32, u32) -> i32;
pub(crate) type Release = unsafe extern "system" fn(Interface) -> u32;

/// D3DVERTEXELEMENT9.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Element {
    pub stream: u16,
    pub offset: u16,
    pub kind: u8,
    pub method: u8,
    pub usage: u8,
    pub usage_index: u8,
}

/// A pre-transformed declaration with its `POSITION` twin and the shader that draws through it.
struct Twin {
    declaration: usize,
    twin: usize,
    shader: usize,
}

struct State {
    shader: usize,
    declaration: usize,
    twins: Vec<Twin>,
}

static STATE: Mutex<State> = Mutex::new(State { shader: 0, declaration: 0, twins: Vec::new() });

pub fn install() {
    let d3d9 = unsafe { GetModuleHandleW(w!("d3d9.dll")) };
    let create = (!d3d9.is_null())
        .then(|| unsafe { GetProcAddress(d3d9, c"Direct3DCreate9".as_ptr().cast()) })
        .flatten();
    match create {
        Some(create) => unsafe { hook(create as *mut c_void, direct3d_create9 as *mut c_void, &ORIGINAL_CREATE, c"Direct3DCreate9") },
        None => log!("d3d9.dll not loaded, menus stay stretched"),
    }
}

pub(crate) unsafe fn hook(target: *mut c_void, detour: *mut c_void, slot: &AtomicUsize, name: &CStr) {
    if slot.load(Ordering::Relaxed) != 0 {
        return;
    }
    let hooked = unsafe {
        MinHook::create_hook(target, detour).and_then(|original| {
            slot.store(original as usize, Ordering::Relaxed);
            MinHook::enable_hook(target)
        })
    };
    match hooked {
        Ok(()) => log!("hooked {}", name.to_string_lossy()),
        Err(e) => log!("cannot hook {}: {e:?}", name.to_string_lossy()),
    }
}

pub(crate) unsafe fn method<T>(object: Interface, index: usize) -> T {
    unsafe { std::mem::transmute_copy(&*(*object).add(index)) }
}

fn original<T>(slot: &AtomicUsize) -> T {
    unsafe { std::mem::transmute_copy(&slot.load(Ordering::Relaxed)) }
}

unsafe extern "system" fn direct3d_create9(version: u32) -> Interface {
    let d3d = unsafe { original::<Direct3DCreate9>(&ORIGINAL_CREATE)(version) };
    if !d3d.is_null() && config::get().widescreen {
        unsafe { hook(method(d3d, CREATE_DEVICE), create_device as *mut c_void, &ORIGINAL_CREATE_DEVICE, c"CreateDevice") };
    }
    d3d
}

unsafe extern "system" fn create_device(
    d3d: Interface,
    adapter: u32,
    device_type: u32,
    window: *mut c_void,
    flags: u32,
    parameters: *mut PresentParameters,
    device: *mut Interface,
) -> i32 {
    let create: CreateDevice = original(&ORIGINAL_CREATE_DEVICE);
    let result = unsafe { create(d3d, adapter, device_type, window, flags, parameters, device) };
    if result >= 0 && !device.is_null() && unsafe { !(*device).is_null() } {
        unsafe { remember_screen(parameters, window) };
        let device = unsafe { *device };
        let hooks: [(usize, *mut c_void, &AtomicUsize, &CStr); 7] = [
            (RESET, reset as *mut c_void, &ORIGINAL_RESET, c"Reset"),
            (SET_FVF, set_fvf as *mut c_void, &ORIGINAL_SET_FVF, c"SetFVF"),
            (CREATE_VERTEX_DECLARATION, create_declaration as *mut c_void, &ORIGINAL_CREATE_DECLARATION, c"CreateVertexDeclaration"),
            (SET_VERTEX_SHADER, set_shader as *mut c_void, &ORIGINAL_SET_SHADER, c"SetVertexShader"),
            (SET_VERTEX_DECLARATION, set_declaration as *mut c_void, &ORIGINAL_SET_DECLARATION, c"SetVertexDeclaration"),
            (DRAW_PRIMITIVE, draw as *mut c_void, &ORIGINAL_DRAW, c"DrawPrimitive"),
            (DRAW_INDEXED_PRIMITIVE, draw_indexed as *mut c_void, &ORIGINAL_DRAW_INDEXED, c"DrawIndexedPrimitive"),
        ];
        for (index, detour, slot, name) in hooks {
            unsafe { hook(method(device, index), detour, slot, name) };
        }
        unsafe { crate::hud::install(device) };
    }
    result
}

/// A resolution change in the game resets the device with the new back buffer size.
unsafe extern "system" fn reset(device: Interface, parameters: *mut PresentParameters) -> i32 {
    let result = unsafe { original::<Reset>(&ORIGINAL_RESET)(device, parameters) };
    if result >= 0 {
        unsafe { remember_screen(parameters, std::ptr::null_mut()) };
        crate::hud::reset();
        if unsafe { parameters.as_ref() }.is_some_and(|p| p.windowed != 0) {
            unsafe { fit_window() };
        }
    }
    result
}

/// The game keeps its window size when the resolution changes, which scales the picture.
unsafe fn fit_window() {
    let window = WINDOW.load(Ordering::Relaxed) as HWND;
    let (width, height) = screen();
    if window.is_null() || width == 0 || height == 0 {
        return;
    }
    let mut rect = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
    unsafe {
        let style = GetWindowLongW(window, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(window, GWL_EXSTYLE) as u32;
        AdjustWindowRectEx(&mut rect, style, 0, ex_style);
        SetWindowPos(
            window,
            std::ptr::null_mut(),
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// A zero back buffer size means the window's client area.
unsafe fn remember_screen(parameters: *const PresentParameters, focus: *mut c_void) {
    let Some(parameters) = (unsafe { parameters.as_ref() }) else { return };
    let (mut width, mut height) = (parameters.back_buffer_width, parameters.back_buffer_height);
    let window = if parameters.device_window.is_null() { focus } else { parameters.device_window };
    if !window.is_null() {
        WINDOW.store(window as usize, Ordering::Relaxed);
    }
    let window = WINDOW.load(Ordering::Relaxed) as HWND;
    if width == 0 || height == 0 {
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        if unsafe { GetClientRect(window, &mut rect) } != 0 {
            (width, height) = ((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32);
        }
    }
    SCREEN.store(u64::from(width) << 32 | u64::from(height), Ordering::Relaxed);
    log!(
        "screen {width}x{height} windowed={} refresh={} interval={:#x}",
        parameters.windowed,
        parameters.refresh_rate,
        parameters.presentation_interval
    );
}

fn screen() -> (u32, u32) {
    let screen = SCREEN.load(Ordering::Relaxed);
    ((screen >> 32) as u32, screen as u32)
}

/// The width a 4:3 image takes on the screen, relative to the screen width.
pub(crate) fn scale_2d() -> f32 {
    let (width, height) = screen();
    if !config::get().widescreen || width == 0 || height == 0 {
        return 1.0;
    }
    (4.0 * height as f32 / (3.0 * width as f32)).min(1.0)
}

unsafe extern "system" fn create_declaration(device: Interface, elements: *const Element, declaration: *mut Interface) -> i32 {
    let create: CreateDeclaration = original(&ORIGINAL_CREATE_DECLARATION);
    let result = unsafe { create(device, elements, declaration) };
    if result < 0 || elements.is_null() {
        return result;
    }
    let Some(count) = (0..=MAX_ELEMENTS).position(|i| unsafe { (*elements.add(i)).stream } == DECLARATION_END_STREAM) else {
        return result;
    };
    let mut elements = unsafe { std::slice::from_raw_parts(elements, count + 1) }.to_vec();
    crate::hud::declaration_created(unsafe { *declaration } as usize, &elements[..count]);
    let Some(position) = elements.iter().position(|e| e.usage == USAGE_POSITIONT && e.usage_index == 0) else {
        return result;
    };
    elements[position].usage = USAGE_POSITION;
    let code = pass_through_shader(&elements[..count], position);
    let mut twin: Interface = std::ptr::null_mut();
    let mut shader: Interface = std::ptr::null_mut();
    let created = unsafe {
        create(device, elements.as_ptr(), &mut twin) >= 0
            && method::<CreateShader>(device, CREATE_VERTEX_SHADER)(device, code.as_ptr(), &mut shader) >= 0
    };
    if !created {
        log!("cannot place 2D declaration {:#x}", unsafe { *declaration } as usize);
    } else if let Ok(mut state) = STATE.lock() {
        state.twins.push(Twin { declaration: unsafe { *declaration } as usize, twin: twin as usize, shader: shader as usize });
    }
    result
}

/// Register the draw hooks fill with the pixel-to-clip mapping (scale x, scale y, offset x, offset y).
const MAPPING_REGISTER: u32 = 95;

/// A vs_1_1 shader doing what the pipeline does with pre-transformed vertices, but with the
/// position mapped through c95; w = 1/rhw keeps textures interpolating as before.
fn pass_through_shader(elements: &[Element], position: usize) -> Vec<u32> {
    let input = |i: usize| 0x9000_0000 | i as u32;
    let mut code = vec![0xfffe_0101];
    for (i, e) in elements.iter().enumerate() {
        code.extend([0x0000_001f, 0x8000_0000 | u32::from(e.usage) | u32::from(e.usage_index) << 16, input(i) | 0x000f_0000]);
    }
    let v = input(position);
    let c = 0xa000_0000 | MAPPING_REGISTER;
    #[rustfmt::skip]
    code.extend([
        0x0000_0006, 0x8008_0001, v | 0x00ff_0000, // rcp r1.w, v.w
        0x0000_0004, 0x8003_0000, v | 0x00e4_0000, c | 0x00e4_0000, c | 0x00ee_0000, // mad r0.xy, v, c95, c95.zwzw
        0x0000_0001, 0x8004_0000, v | 0x00e4_0000, // mov r0.z, v
        0x0000_0005, 0xc007_0000, 0x80e4_0000, 0x80ff_0001, // mul oPos.xyz, r0, r1.w
        0x0000_0001, 0xc008_0000, 0x80ff_0001, // mov oPos.w, r1.w
    ]);
    for (i, e) in elements.iter().enumerate() {
        let output = match (e.usage, e.usage_index) {
            (USAGE_COLOR, index @ 0..=1) => 0xd00f_0000 | u32::from(index),
            (USAGE_TEXCOORD, index @ 0..=7) => 0xe00f_0000 | u32::from(index),
            _ => continue,
        };
        code.extend([0x0000_0001, output, input(i) | 0x00e4_0000]);
    }
    code.push(0x0000_ffff);
    code
}

unsafe extern "system" fn set_shader(device: Interface, shader: Interface) -> i32 {
    if let Ok(mut state) = STATE.lock() {
        state.shader = shader as usize;
    }
    unsafe { original::<SetObject>(&ORIGINAL_SET_SHADER)(device, shader) }
}

unsafe extern "system" fn set_declaration(device: Interface, declaration: Interface) -> i32 {
    if let Ok(mut state) = STATE.lock() {
        state.declaration = declaration as usize;
    }
    unsafe { original::<SetObject>(&ORIGINAL_SET_DECLARATION)(device, declaration) }
}

unsafe extern "system" fn set_fvf(device: Interface, fvf: u32) -> i32 {
    // The game follows each declaration with SetFVF(0), which keeps the declaration.
    if fvf != 0
        && let Ok(mut state) = STATE.lock()
    {
        state.declaration = 0;
    }
    unsafe { original::<unsafe extern "system" fn(Interface, u32) -> i32>(&ORIGINAL_SET_FVF)(device, fvf) }
}

unsafe extern "system" fn draw(device: Interface, kind: u32, start: u32, count: u32) -> i32 {
    let declaration = STATE.lock().map_or(0, |state| state.declaration);
    unsafe {
        crate::hud::draw(device, declaration, kind, start, count, || {
            draw_2d(device, || original::<Draw>(&ORIGINAL_DRAW)(device, kind, start, count))
        })
    }
}

unsafe extern "system" fn draw_indexed(
    device: Interface,
    kind: u32,
    base: i32,
    min: u32,
    vertices: u32,
    start: u32,
    count: u32,
) -> i32 {
    unsafe {
        draw_2d(device, || original::<DrawIndexed>(&ORIGINAL_DRAW_INDEXED)(device, kind, base, min, vertices, start, count))
    }
}

/// Runs a draw; a pre-transformed one goes through its twin with the pixel mapping in c95.
unsafe fn draw_2d(device: Interface, draw: impl FnOnce() -> i32) -> i32 {
    if scale_2d() >= 1.0 {
        return draw();
    }
    let found = STATE.lock().ok().and_then(|state| {
        let twin = state.twins.iter().find(|t| t.declaration == state.declaration)?;
        Some((state.declaration, state.shader, twin.twin, twin.shader))
    });
    let Some((declaration, shader, twin, twin_shader)) = found else { return draw() };
    let Some((width, height)) = (unsafe { target_size(device) }) else { return draw() };
    let on_screen = is_screen(width, height);
    let squeeze = if on_screen && !unsafe { fills_screen(device) } { scale_2d() } else { 1.0 };
    let mut mapping = [2.0 * squeeze / width as f32, -2.0 / height as f32, -squeeze, 1.0];
    let mut saved = [0.0f32; 4];
    unsafe {
        let get_constants: ShaderConstants = method(device, GET_VERTEX_SHADER_CONSTANT_F);
        let set_constants: ShaderConstants = method(device, SET_VERTEX_SHADER_CONSTANT_F);
        let set_declaration: SetObject = original(&ORIGINAL_SET_DECLARATION);
        let set_shader: SetObject = original(&ORIGINAL_SET_SHADER);
        // c95 may hold the last bone of a skinned draw whose bones the game will not upload again.
        get_constants(device, MAPPING_REGISTER, saved.as_mut_ptr(), 1);
        set_constants(device, MAPPING_REGISTER, mapping.as_mut_ptr(), 1);
        set_declaration(device, twin as Interface);
        set_shader(device, twin_shader as Interface);
        let result = draw();
        set_shader(device, shader as Interface);
        set_declaration(device, declaration as Interface);
        set_constants(device, MAPPING_REGISTER, saved.as_mut_ptr(), 1);
        result
    }
}

/// Untextured quads are fades and tints, render textures the 3D view and its effects: both cover the
/// whole screen, unlike menus, HUD and videos.
unsafe fn fills_screen(device: Interface) -> bool {
    type GetTexture = unsafe extern "system" fn(Interface, u32, *mut Interface) -> i32;
    type GetType = unsafe extern "system" fn(Interface) -> u32;
    type GetLevelDesc = unsafe extern "system" fn(Interface, u32, *mut [u32; 8]) -> i32;
    let mut texture: Interface = std::ptr::null_mut();
    unsafe {
        if method::<GetTexture>(device, GET_TEXTURE)(device, 0, &mut texture) < 0 || texture.is_null() {
            return true;
        }
        // D3DSURFACE_DESC: Format, Type, Usage, Pool, MultiSampleType, MultiSampleQuality, Width, Height.
        let mut desc = [0u32; 8];
        let described = method::<GetType>(texture, RESOURCE_GET_TYPE)(texture) == RESOURCE_TYPE_TEXTURE
            && method::<GetLevelDesc>(texture, TEXTURE_GET_LEVEL_DESC)(texture, 0, &mut desc) >= 0;
        method::<Release>(texture, RELEASE)(texture);
        described && desc[2] & USAGE_RENDER_TARGET != 0
    }
}

fn is_screen(width: u32, height: u32) -> bool {
    (width, height) == screen()
}

pub(crate) unsafe fn draws_on_screen(device: Interface) -> bool {
    unsafe { target_size(device) }.is_some_and(|(width, height)| is_screen(width, height))
}

unsafe fn target_size(device: Interface) -> Option<(u32, u32)> {
    type GetRenderTarget = unsafe extern "system" fn(Interface, u32, *mut Interface) -> i32;
    type GetDesc = unsafe extern "system" fn(Interface, *mut [u32; 8]) -> i32;
    let mut surface: Interface = std::ptr::null_mut();
    unsafe {
        if method::<GetRenderTarget>(device, GET_RENDER_TARGET)(device, 0, &mut surface) < 0 || surface.is_null() {
            return None;
        }
        let mut desc = [0u32; 8];
        let described = method::<GetDesc>(surface, SURFACE_GET_DESC)(surface, &mut desc) >= 0;
        method::<Release>(surface, RELEASE)(surface);
        described.then_some((desc[6], desc[7]))
    }
}
