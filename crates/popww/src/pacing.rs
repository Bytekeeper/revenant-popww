//! Frame pacing: the engine steps by the time it measures between two frames, which jitters by a few
//! milliseconds even when every frame reaches the screen exactly one refresh apart. With VSync on, the
//! game clock instead advances in whole refresh periods and only follows real time on average.

/// Frames the period is averaged over and the share of them that must be near one refresh.
const WINDOW: usize = 256;
const RECENT: usize = 32;
const STEADY: usize = 28;
/// How far a frame may run late before it counts as a missed refresh.
const LATE: f64 = 0.7;
/// The share of the gap to real time that each frame closes.
const CORRECTION: f64 = 1.0 / 32.0;

pub struct Pacer {
    period: f64,
    last_real: Option<f64>,
    paced: f64,
    /// Real time not yet passed to the game; negative when the game is ahead.
    behind: f64,
    /// (measured frame time, refreshes it was given) of the latest frames.
    frames: std::collections::VecDeque<(f64, u32)>,
}

impl Pacer {
    /// `period` is the display's refresh period, in the same unit as the times.
    pub fn new(period: f64) -> Self {
        Self { period, last_real: None, paced: 0.0, behind: 0.0, frames: Default::default() }
    }

    /// The game time for a frame that starts at `real`.
    pub fn time(&mut self, real: f64) -> f64 {
        let Some(last_real) = self.last_real.replace(real) else {
            self.paced = real;
            return real;
        };
        let elapsed = real - last_real;
        let refreshes = ((self.behind + elapsed) / self.period + 1.0 - LATE).floor().max(1.0);
        self.remember(elapsed, refreshes as u32);
        if !self.steady() {
            self.behind = 0.0;
            self.paced = real;
            return real;
        }
        self.behind += elapsed;
        let step = refreshes * self.period + CORRECTION * (self.behind - refreshes * self.period);
        self.behind -= step;
        self.paced += step;
        self.paced
    }

    fn remember(&mut self, elapsed: f64, refreshes: u32) {
        if self.frames.len() == WINDOW {
            self.frames.pop_front();
        }
        self.frames.push_back((elapsed, refreshes));
        if self.frames.len() == WINDOW && self.steady() {
            let (time, count) = self.frames.iter().fold((0.0, 0), |(t, n), &(e, r)| (t + e, n + r));
            self.period = time / f64::from(count);
        }
    }

    /// Whether nearly all recent frames took about one refresh, as with VSync at full frame rate.
    fn steady(&self) -> bool {
        let near = |&&(elapsed, _): &&(f64, u32)| (0.5..1.5).contains(&(elapsed / self.period));
        self.frames.len() >= RECENT && self.frames.iter().rev().take(RECENT).filter(near).count() >= STEADY
    }
}

#[cfg(windows)]
pub use patch::install;

#[cfg(windows)]
mod patch {
    use std::sync::Mutex;

    use windows_sys::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};

    use super::Pacer;
    use crate::game::{matches, write};
    use crate::log::log;

    /// The frame clock's `call __aulldiv`, which turns CPU ticks into microseconds.
    const CLOCK_CALL: usize = 0x42122d;
    const CLOCK_CALL_ORIGINAL: &[u8] = &[0xe8, 0xee, 0x44, 0x42, 0x00];
    /// The frame clock rounds frame times to this many microseconds; 1 keeps the paced times exact.
    const QUANTUM_LOAD: usize = 0x4212ac;
    const QUANTUM_LOAD_ORIGINAL: &[u8] = &[0x8b, 0x1d, 0x48, 0x15, 0x91, 0x00];
    const QUANTUM_LOAD_ONE: &[u8] = &[0xbb, 0x01, 0x00, 0x00, 0x00, 0x90];

    static PACER: Mutex<Option<Pacer>> = Mutex::new(None);

    pub fn install() {
        if !unsafe { matches(CLOCK_CALL, CLOCK_CALL_ORIGINAL) && matches(QUANTUM_LOAD, QUANTUM_LOAD_ORIGINAL) } {
            log!("not patched, unknown code: frame pacing");
            return;
        }
        let hz = refresh_rate();
        *PACER.lock().unwrap() = Some(Pacer::new(1e6 / hz));
        let target = clock as *const () as isize - (CLOCK_CALL as isize + 5);
        let mut call = [0xe8u8, 0, 0, 0, 0];
        call[1..].copy_from_slice(&(target as i32).to_le_bytes());
        unsafe {
            write(QUANTUM_LOAD, QUANTUM_LOAD_ONE);
            write(CLOCK_CALL, &call);
        }
        log!("patched: frame pacing, starting at {hz} Hz");
    }

    /// Stands in for `__aulldiv(ticks, ticks_per_microsecond)` in the frame clock.
    unsafe extern "stdcall" fn clock(ticks: u64, ticks_per_microsecond: u64) -> u64 {
        let real = ticks as f64 / ticks_per_microsecond.max(1) as f64;
        match PACER.lock().unwrap().as_mut() {
            Some(pacer) => pacer.time(real).round() as u64,
            None => real as u64,
        }
    }

    fn refresh_rate() -> f64 {
        let mut mode: DEVMODEW = unsafe { std::mem::zeroed() };
        mode.dmSize = size_of::<DEVMODEW>() as u16;
        let hz = if unsafe { EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut mode) } != 0 {
            mode.dmDisplayFrequency
        } else {
            0
        };
        // 0 and 1 mean the hardware default; the measured frames correct the guess.
        if hz > 1 { f64::from(hz) } else { 60.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::Pacer;

    const PERIOD: f64 = 1e6 / 69.87;

    /// Frame starts one refresh apart, measured up to `jitter` microseconds early or late.
    fn vsynced(frames: usize, jitter: f64) -> Vec<f64> {
        let mut seed = 12345u32;
        (0..frames)
            .map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                i as f64 * PERIOD + (f64::from(seed >> 8) / f64::from(1u32 << 24) * 2.0 - 1.0) * jitter
            })
            .collect()
    }

    fn steps(pacer: &mut Pacer, times: &[f64]) -> Vec<f64> {
        let paced: Vec<f64> = times.iter().map(|&t| pacer.time(t)).collect();
        paced.windows(2).map(|w| w[1] - w[0]).collect()
    }

    #[test]
    fn vsync_jitter_becomes_even_steps() {
        let mut pacer = Pacer::new(1e6 / 70.0);
        let times = vsynced(2000, 4000.0);
        let steps = steps(&mut pacer, &times);
        for step in &steps[600..] {
            assert!((step - PERIOD).abs() < 200.0, "step {step}");
        }
        assert!((pacer.paced - times[1999]).abs() < PERIOD, "game time follows real time");
    }

    #[test]
    fn a_missed_refresh_is_a_double_step() {
        let mut pacer = Pacer::new(PERIOD);
        let mut times = vsynced(400, 1000.0);
        times.remove(300);
        let steps = steps(&mut pacer, &times);
        assert!((steps[299] - 2.0 * PERIOD).abs() < 300.0, "step {}", steps[299]);
        assert!((steps[300] - PERIOD).abs() < 300.0, "step {}", steps[300]);
    }

    #[test]
    fn without_vsync_real_time_passes_through() {
        let mut pacer = Pacer::new(PERIOD);
        let times: Vec<f64> = (0..500).map(|i| i as f64 * 4100.0 + (i % 3) as f64 * 700.0).collect();
        let paced: Vec<f64> = times.iter().map(|&t| pacer.time(t)).collect();
        assert_eq!(paced, times);
    }

    #[test]
    fn a_long_frame_ends_pacing_only_briefly() {
        let mut pacer = Pacer::new(PERIOD);
        let mut times = vsynced(600, 1000.0);
        for t in &mut times[300..] {
            *t += 500_000.0;
        }
        let steps = steps(&mut pacer, &times);
        assert!(steps[299] > 400_000.0);
        assert!((steps[400] - PERIOD).abs() < 300.0, "step {}", steps[400]);
    }
}
