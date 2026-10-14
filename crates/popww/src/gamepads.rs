//! New profiles take their gamepad bindings from `gamepads.dat`, which knows no Xbox pads; the
//! game's fallback for unknown pads puts block on Start. The game reads an extended copy instead.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::log::log;

const RECORD: usize = 32;
const NONE: u8 = 0xff;

/// Xbox 360, One and Series pads and Steam's virtual pad, which DirectInput all shows like a 360 pad.
const XBOX_PADS: &[(u16, u16)] = &[
    (0x045e, 0x028e),
    (0x045e, 0x0719),
    (0x045e, 0x02d1),
    (0x045e, 0x02dd),
    (0x045e, 0x02e3),
    (0x045e, 0x02ea),
    (0x045e, 0x0b00),
    (0x045e, 0x0b05),
    (0x045e, 0x0b12),
    (0x045e, 0x0b13),
    (0x045e, 0x0b20),
    (0x045e, 0x0b22),
    (0x28de, 0x11ff),
];

/// DirectInput buttons of a 360 pad (A, B, X, Y, LB, RB, Back, Start, left and right stick click),
/// or the triggers, which share the Z axis: left pushes it up, right down.
const LEFT_TRIGGER: u8 = 0x25;
const RIGHT_TRIGGER: u8 = 0x24;
const BUTTONS: [u8; 12] = [
    0,             // jump, roll
    1,             // pick up, throw weapon
    2,             // attack
    3,             // secondary weapon
    NONE,          // landscape camera
    8,             // first-person look
    LEFT_TRIGGER,  // rewind
    RIGHT_TRIGGER, // block, special action
    9,             // reset camera
    NONE,          // unused on the keyboard too
    7,             // pause menu
    6,             // Tab view
];
/// The d-pad on the hat switch, the left stick on X/Y and the right stick on X/Y rotation.
const AXES: [u8; 12] = [0x30, 0x33, 0x31, 0x32, 0x22, 0x21, 0x23, 0x20, 0x28, 0x26, 0x29, 0x27];

/// The game's `gamepads.dat` (the case-corrected path) with records for the pads it lacks.
pub fn extended(original: &Path) -> Option<&'static Path> {
    static EXTENDED: OnceLock<Option<PathBuf>> = OnceLock::new();
    EXTENDED.get_or_init(|| write_extended(original)).as_deref()
}

fn write_extended(original: &Path) -> Option<PathBuf> {
    let mut table = std::fs::read(original).unwrap_or_default();
    table.truncate(table.len() / RECORD * RECORD);
    let known: Vec<(u16, u16)> = table
        .chunks_exact(RECORD)
        .map(|r| (u16::from_le_bytes([r[0], r[1]]), u16::from_le_bytes([r[2], r[3]])))
        .collect();
    for &(vendor, product) in XBOX_PADS.iter().filter(|pad| !known.contains(pad)) {
        table.extend(vendor.to_le_bytes());
        table.extend(product.to_le_bytes());
        table.extend(1.0f32.to_le_bytes());
        table.extend(BUTTONS);
        table.extend(AXES);
    }
    let path = std::env::temp_dir().join("popww-gamepads.dat");
    match std::fs::write(&path, &table) {
        Ok(()) => {
            log!("gamepad table with Xbox pads: {}", path.display());
            Some(path)
        }
        Err(e) => {
            log!("cannot write {}: {e}", path.display());
            None
        }
    }
}

/// Whether the game opens its gamepad table.
pub fn is_table(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name.eq_ignore_ascii_case("gamepads.dat"))
}
