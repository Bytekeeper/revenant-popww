//! The HUD (health, sand tanks) is geometry the engine fits to fixed screen positions whatever the
//! aspect ratio, which stretches it on wider screens. Each HUD draw is narrowed back to 4:3
//! proportions towards the screen edge it is near, or towards the middle.
//!
//! The vertices are written by the CPU into write-only buffers, so the HUD's buffers are shadowed
//! as the game unlocks them, to find where each draw lands on the screen.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::d3d::{self, Interface, method};

const CREATE_VERTEX_BUFFER: usize = 26;
const GET_STREAM_SOURCE: usize = 101;
const BUFFER_LOCK: usize = 11;
const BUFFER_UNLOCK: usize = 12;
const BUFFER_GET_DESC: usize = 13;
const POOL_DEFAULT: u32 = 0;
const TYPE_FLOAT3: u8 = 2;
const TYPE_FLOAT4: u8 = 3;
/// Normalized x beyond which a draw counts as reaching the screen edge.
const FULL_WIDTH: f32 = 0.99;
const MAX_BUFFER_SIZE: usize = 4 << 20;

static ORIGINAL_SET_CONSTANTS: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_LOCK: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_UNLOCK: AtomicUsize = AtomicUsize::new(0);

type Lock = unsafe extern "system" fn(Interface, u32, u32, *mut *mut u8, u32) -> i32;
type Unlock = unsafe extern "system" fn(Interface) -> i32;

struct Shadow {
    buffer: usize,
    data: Vec<u8>,
    /// The game's pointer and range while the buffer is locked.
    locked: Option<(usize, usize, usize)>,
}

struct State {
    /// The vertex shader's matrix registers c0-c3; a pure device cannot report them.
    matrix: [f32; 16],
    /// Declarations with a float position, by stream and offset.
    positions: Vec<(usize, u16, u16)>,
    shadows: Vec<Shadow>,
}

static STATE: Mutex<State> = Mutex::new(State { matrix: [0.0; 16], positions: Vec::new(), shadows: Vec::new() });

/// Hooks the vertex buffer methods, which all buffers share, through a throwaway buffer.
pub unsafe fn install(device: Interface) {
    type CreateBuffer = unsafe extern "system" fn(Interface, u32, u32, u32, u32, *mut Interface, *mut c_void) -> i32;
    let mut buffer: Interface = std::ptr::null_mut();
    unsafe {
        let create: CreateBuffer = method(device, CREATE_VERTEX_BUFFER);
        if create(device, 64, 0, 0, POOL_DEFAULT, &mut buffer, std::ptr::null_mut()) < 0 || buffer.is_null() {
            crate::log::log!("cannot create a vertex buffer, HUD stays in the middle");
            return;
        }
        d3d::hook(
            method(device, d3d::SET_VERTEX_SHADER_CONSTANT_F),
            set_constants as *mut c_void,
            &ORIGINAL_SET_CONSTANTS,
            c"SetVertexShaderConstantF",
        );
        d3d::hook(method(buffer, BUFFER_LOCK), lock as *mut c_void, &ORIGINAL_LOCK, c"IDirect3DVertexBuffer9::Lock");
        d3d::hook(method(buffer, BUFFER_UNLOCK), unlock as *mut c_void, &ORIGINAL_UNLOCK, c"IDirect3DVertexBuffer9::Unlock");
        method::<d3d::Release>(buffer, d3d::RELEASE)(buffer);
    }
}

pub fn declaration_created(declaration: usize, elements: &[d3d::Element]) {
    let position = elements
        .iter()
        .find(|e| e.usage == d3d::USAGE_POSITION && e.usage_index == 0 && matches!(e.kind, TYPE_FLOAT3 | TYPE_FLOAT4));
    if let Some(e) = position
        && let Ok(mut state) = STATE.lock()
    {
        state.positions.retain(|&(d, _, _)| d != declaration);
        state.positions.push((declaration, e.stream, e.offset));
    }
}

unsafe extern "system" fn set_constants(device: Interface, start: u32, data: *mut f32, count: u32) -> i32 {
    if start < 4
        && !data.is_null()
        && let Ok(mut state) = STATE.lock()
    {
        let registers = count.min(4 - start) as usize;
        let values = unsafe { std::slice::from_raw_parts(data, 4 * registers) };
        state.matrix[4 * start as usize..][..4 * registers].copy_from_slice(values);
    }
    unsafe { std::mem::transmute::<usize, d3d::ShaderConstants>(ORIGINAL_SET_CONSTANTS.load(Ordering::Relaxed))(device, start, data, count) }
}

/// A device reset recreates the game's dynamic buffers, possibly at the old addresses.
pub fn reset() {
    // A locked shadow is still the game's write target.
    if let Ok(mut state) = STATE.lock() {
        state.shadows.retain(|s| s.locked.is_some());
    }
}

/// The game writes into the shadow and the unlock copies its range to the buffer: reading the
/// buffer back is slow where it lives in GPU-visible memory, as with DXVK.
unsafe extern "system" fn lock(buffer: Interface, offset: u32, size: u32, data: *mut *mut u8, flags: u32) -> i32 {
    let result = unsafe { std::mem::transmute::<usize, Lock>(ORIGINAL_LOCK.load(Ordering::Relaxed))(buffer, offset, size, data, flags) };
    if result >= 0
        && !data.is_null()
        && let Ok(mut state) = STATE.lock()
        && let Some(shadow) = state.shadows.iter_mut().find(|s| s.buffer == buffer as usize)
        && (offset as usize) < shadow.data.len()
    {
        let (start, end) = (offset as usize, if size == 0 { shadow.data.len() } else { (offset + size) as usize });
        shadow.locked = Some((unsafe { *data } as usize, start, end.min(shadow.data.len())));
        unsafe { *data = shadow.data.as_mut_ptr().add(start) };
    }
    result
}

unsafe extern "system" fn unlock(buffer: Interface) -> i32 {
    if let Ok(mut state) = STATE.lock()
        && let Some(shadow) = state.shadows.iter_mut().find(|s| s.buffer == buffer as usize)
        && let Some((pointer, start, end)) = shadow.locked.take()
    {
        unsafe { std::ptr::copy_nonoverlapping(shadow.data.as_ptr().add(start), pointer as *mut u8, end - start) };
    }
    unsafe { std::mem::transmute::<usize, Unlock>(ORIGINAL_UNLOCK.load(Ordering::Relaxed))(buffer) }
}

/// Runs a draw; a HUD draw is narrowed to 4:3 proportions.
pub unsafe fn draw(device: Interface, declaration: usize, kind: u32, start: u32, count: u32, draw: impl FnOnce() -> i32) -> i32 {
    let scale = d3d::scale_2d();
    if scale >= 1.0 {
        return draw();
    }
    let Ok(mut matrix) = STATE.lock().map(|state| state.matrix) else { return draw() };
    if !is_hud_camera(&matrix) || !unsafe { d3d::draws_on_screen(device) } {
        return draw();
    }
    let Some((left, right)) = (unsafe { screen_extent(device, declaration, kind, start, count, &matrix) }) else { return draw() };
    // Backgrounds such as the pause menu's tint cover the whole screen.
    if left <= -FULL_WIDTH && right >= FULL_WIDTH {
        return draw();
    }
    let centre = (left + right) / 2.0;
    // Narrowed to 4:3 proportions towards the nearest screen edge, or the middle.
    let anchor = if centre < -1.0 / 3.0 {
        scale - 1.0
    } else if centre > 1.0 / 3.0 {
        1.0 - scale
    } else {
        0.0
    };
    // x/w becomes scale * x/w + anchor.
    let mut moved: [f32; 4] = std::array::from_fn(|i| scale * matrix[i] + anchor * matrix[12 + i]);
    unsafe {
        let set: d3d::ShaderConstants = std::mem::transmute(ORIGINAL_SET_CONSTANTS.load(Ordering::Relaxed));
        set(device, 0, moved.as_mut_ptr(), 1);
        let result = draw();
        set(device, 0, matrix.as_mut_ptr(), 1);
        result
    }
}

/// The HUD camera sits at the origin without rotation: x depends on x alone and w has no offset.
fn is_hud_camera(m: &[f32; 16]) -> bool {
    m[0] != 0.0 && m[1] == 0.0 && m[2] == 0.0 && m[3] == 0.0 && m[15] == 0.0
}

/// The draw's horizontal extent in normalized screen coordinates.
unsafe fn screen_extent(device: Interface, declaration: usize, kind: u32, start: u32, count: u32, m: &[f32; 16]) -> Option<(f32, f32)> {
    type GetStreamSource = unsafe extern "system" fn(Interface, u32, *mut Interface, *mut u32, *mut u32) -> i32;
    type GetDesc = unsafe extern "system" fn(Interface, *mut [u32; 6]) -> i32;
    let (stream, position) = STATE.lock().ok()?.positions.iter().find(|p| p.0 == declaration).map(|p| (p.1, p.2))?;
    let mut buffer: Interface = std::ptr::null_mut();
    let (mut offset, mut stride) = (0u32, 0u32);
    unsafe {
        let get: GetStreamSource = method(device, GET_STREAM_SOURCE);
        if get(device, u32::from(stream), &mut buffer, &mut offset, &mut stride) < 0 || buffer.is_null() {
            return None;
        }
        method::<d3d::Release>(buffer, d3d::RELEASE)(buffer);
    }
    let mut state = STATE.lock().ok()?;
    let Some(shadow) = state.shadows.iter().find(|s| s.buffer == buffer as usize) else {
        // Shadowed from the next write on; this draw stays where it is.
        let mut desc = [0u32; 6];
        let described = unsafe { method::<GetDesc>(buffer, BUFFER_GET_DESC)(buffer, &mut desc) } >= 0;
        let size = desc[4] as usize;
        if described && size <= MAX_BUFFER_SIZE {
            state.shadows.push(Shadow { buffer: buffer as usize, data: vec![0; size], locked: None });
        }
        return None;
    };
    let vertices = match kind {
        1 => count,
        2 => 2 * count,
        3 => count + 1,
        4 => 3 * count,
        5 | 6 => count + 2,
        _ => return None,
    };
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for i in start..start + vertices {
        let at = offset as usize + i as usize * stride as usize + usize::from(position);
        let p: [f32; 3] = std::array::from_fn(|k| {
            let bytes = shadow.data.get(at + 4 * k..at + 4 * k + 4).map_or([0; 4], |b| b.try_into().unwrap_or([0; 4]));
            f32::from_le_bytes(bytes)
        });
        let w = m[12] * p[0] + m[13] * p[1] + m[14] * p[2];
        if w <= 0.0 {
            return None;
        }
        let x = (m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3]) / w;
        low = low.min(x);
        high = high.max(x);
    }
    (low <= high).then_some((low, high))
}
