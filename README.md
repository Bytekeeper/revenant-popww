# popww: a modern fix for Prince of Persia: Warrior Within

Copy two files into the game folder and play *Prince of Persia: Warrior Within* (PC, 2004) at any
resolution, in widescreen, without the old launcher. Works on Windows and on Linux through Wine or
Proton.

## What it fixes

- **Starts without the launcher**: no hardware detection dialog and no "start the game with the
  launcher" error.
- **Any resolution** your graphics card offers, fullscreen or windowed; the desktop resolution by
  default.
- **Widescreen (Hor+)**: wider screens show more at the sides instead of a stretched or cropped
  picture. Menus and videos keep their 4:3 proportions; the HUD stays round and sits at the
  screen corners.
- **Shorter waits**: the logo videos are left out, cinematics end with Esc, Space, Enter or gamepad
  A/B/Start, and loading screens end as soon as the level is loaded.
- **Xbox controllers** (360, One, Series, Steam's virtual pad) get a console-like layout in new
  profiles: A jump, X attack, Y secondary weapon, B pick up/throw, RT block, LT rewind, Start
  pause. Existing profiles keep their bindings; change them under Options > Controls.
- **Wine/Proton**: file names are matched regardless of case, and the white menu background is gone.

## Install

1. Download `popww-<version>.zip` from the [releases](../../releases).
2. Unpack `PrinceOfPersia.exe` and `popww.dll` into the game folder (where `pop2.exe` is), replacing
   the original `PrinceOfPersia.exe`. Keep a copy of the original if you want to go back.
3. Start the game as usual: from GOG Galaxy, Heroic, Lutris or by running `PrinceOfPersia.exe`.

Tested with the GOG version (1.0.0.188). Other versions start but may lack some of the fixes;
`popww.log` in the game folder says which were applied.

## Settings

`popww.ini` appears in the game folder on the first start:

| Setting | Default | Meaning |
| --- | --- | --- |
| `[Display] Width`, `Height` | `0` | Resolution; `0` uses the desktop resolution. |
| `[Display] Fullscreen` | `1` | `0` plays in a window. |
| `[Display] Widescreen` | `1` | `0` keeps the original look: cropped view, stretched menus and HUD. |
| `[Video] SkipLogos` | `1` | `0` shows the logo videos at startup. |

The file also explains how to override the hardware description the old launcher used to write.

## Building

Needs Rust and the 32-bit MinGW toolchain (`i686-w64-mingw32-gcc`):

```sh
rustup target add i686-pc-windows-gnu
cargo build --release
```

The files land in `target/i686-pc-windows-gnu/release/`. Pushing a `v*` tag builds a release on GitHub.

## License

MIT. *Prince of Persia* is a trademark of Ubisoft; this project is not affiliated with Ubisoft and
contains no game files.
