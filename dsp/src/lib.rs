//! Seedspace engine. No plugin-framework imports, so the wrapper stays swappable.
#![allow(clippy::needless_range_loop)]

use std::f32::consts::{FRAC_PI_4, PI};

pub const LINES: usize = 8;
pub const COMBS: usize = 6;

const GRID: [f32; LINES] = [
    1.0,
    3.0 / 4.0,
    2.0 / 3.0,
    1.0 / 2.0,
    3.0 / 8.0,
    1.0 / 3.0,
    1.0 / 4.0,
    5.0 / 8.0,
];
const PRISM_RATIOS: [f32; 6] = [2.0, 1.498_307, 0.5, 1.334_84, 2.996_614, -1.0];
const MINOR_PENTATONIC: [i32; 5] = [0, 3, 5, 7, 10];
// Hadamard rows are orthonormal, so the shimmer and tune paths can never add energy to each other.
const SHIMMER_ROWS: [usize; 2] = [1, 2];
const TUNE_ROWS: [usize; COMBS] = [0, 3, 4, 5, 6, 7];

const DETUNE: f32 = 0.04;
const MAX_SIZE_S: f32 = 2.0;
const BLOOM_STRETCH: f32 = 0.5;
const BLOOM_DUCK: f32 = 0.8;
const BLOOM_SENS: f32 = 2.0;
const MOTION_S: f32 = 0.02;
const SCATTER_RATE: f32 = 20.0;
const SCATTER_MIN_SCALE: f32 = 0.1;
const HEAD_FADE_MS: f32 = 5.0;
const DIFFUSER_STEPS: usize = 3;
const DIFFUSER_MAX_MS: f32 = 20.0;
const PRISM_MS: f32 = 60.0;
const REVERSE_MS: f32 = 180.0;
const PRISM_FADE: f32 = 0.25;
// Random grain start, as a share of the window.
const SHIFT_JITTER: f32 = 0.25;
const TUNE_MAX: f32 = 0.3;
// A comb at peak gain 1 only returns what its row carried, so the chord needs a path out of the loop.
const TUNE_OUT: f32 = 16.0;
// Per comb, because a steady tone exactly on a note would otherwise jump 18 dB out of the wet.
const TUNE_CEILING: f32 = 0.25;
const TUNE_MIN_HZ: f32 = 50.0;
// Comb ring in periods: a fixed count keeps the resonance about ±20 cents wide on every note.
const TUNE_PERIODS: f32 = 90.0;
const TUNE_HARMONICS: f32 = 4.0;
const CONTROL_PERIOD: u32 = 32;
// Lines run 12 dB under the input and the output makes it up, so program never reaches the knee.
const IN_SCALE: f32 = 0.25;
const OUT_GAIN: f32 = 2.0;
const LIMIT_KNEE: f32 = 0.5;
// Share of the shifter cycle spent crossfading.
const SHIFT_FADE: f32 = 0.3;
// Average power of two uncorrelated taps: 3/4 inside sin² crossfades, 1 outside.
const SHIFT_ENERGY: f32 = 1.0 - 0.25 * SHIFT_FADE;
// Measured: rows mix only partly per pass, while Prism also cascades out through Tone and Low Cut.
const DECAY_MAKEUP: f32 = 0.7;
const PRISM_MAKEUP: f32 = 3.0;
// A mode the sounds never touch rings at most this many times Decay.
const MAKEUP_RING: f32 = 4.0;
// Full compensation flattens sustained level but buries hits at long Decay; 0.4 splits the difference.
const COMP_DEPTH: f32 = 0.4;

/// Control snapshot in plain units. The wrapper resolves tempo sync into `size_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineParams {
    /// 10..2000, length of the longest line
    pub size_ms: f32,
    /// 0.1..30, T60
    pub decay_s: f32,
    /// 0 = independent echoes, 1 = dense reverb
    pub diffuse: f32,
    /// 500..20000
    pub tone_hz: f32,
    /// 20..1000
    pub low_cut_hz: f32,
    /// 0 = mono, 1 = full
    pub width: f32,
    pub freeze: bool,
    /// 0..=9999
    pub seed: u32,
    pub geometry: f32,
    pub material: f32,
    pub motion: f32,
    pub scatter: f32,
    pub prism: f32,
    pub shimmer: f32,
    pub tune: f32,
    /// Semitones above C, 0..12
    pub key: u8,
    pub bloom: f32,
}

impl Default for EngineParams {
    fn default() -> Self {
        Self {
            size_ms: 120.0,
            decay_s: 3.0,
            diffuse: 0.85,
            tone_hz: 8000.0,
            low_cut_hz: 80.0,
            width: 1.0,
            freeze: false,
            seed: 1234,
            geometry: 0.6,
            material: 0.3,
            motion: 0.15,
            scatter: 0.0,
            prism: 0.0,
            shimmer: 0.0,
            tune: 0.0,
            key: 0,
            bloom: 0.0,
        }
    }
}

fn splitmix32(x: u32) -> u32 {
    let mut z = x.wrapping_add(0x9E37_79B9);
    z = (z ^ (z >> 16)).wrapping_mul(0x85EB_CA6B);
    z = (z ^ (z >> 13)).wrapping_mul(0xC2B2_AE35);
    z ^ (z >> 16)
}

struct Rng(u32);

impl Rng {
    /// Hashed first because a xorshift state of 0 outputs 0 forever.
    fn new(seed: u32) -> Self {
        Self(splitmix32(seed) | 1)
    }

    /// xorshift32, uniform in [0, 1)
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }

    fn bipolar(&mut self) -> f32 {
        self.next() * 2.0 - 1.0
    }

    fn shuffled<const N: usize>(&mut self) -> [usize; N] {
        let mut a: [usize; N] = std::array::from_fn(|i| i);
        for i in (1..N).rev() {
            a.swap(i, (self.next() * (i + 1) as f32) as usize);
        }
        a
    }
}

/// Hidden structure of one space, pure function of the seed. Also drawn by the GUI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceDna {
    /// Per-line damping offset in -1..1, scaled by Material
    pub damp: [f32; LINES],
    /// Prism pitch ratio per line, -1 means reverse
    pub prism_ratio: [f32; LINES],
    /// Prism knob position where each line starts to enter
    pub prism_entry: [f32; LINES],
    /// Stereo position per line in -1..1
    pub pan: [f32; LINES],
    random_ratio: [f32; LINES],
    detune: [f32; LINES],
    motion_rate: [f32; LINES],
    diffuser_ms: [[f32; LINES]; DIFFUSER_STEPS],
    diffuser_sign: [[f32; LINES]; DIFFUSER_STEPS],
    tune_degree: [usize; COMBS],
}

impl SpaceDna {
    pub fn from_seed(seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        let log_bin: [usize; LINES] = rng.shuffled();
        let pan_slot: [usize; LINES] = rng.shuffled();
        let entry_slot: [usize; LINES] = rng.shuffled();
        let degrees: [usize; 16] = rng.shuffled();
        let mut dna = Self {
            damp: [0.0; LINES],
            prism_ratio: [0.0; LINES],
            prism_entry: entry_slot.map(|slot| slot as f32 / (LINES - 1) as f32 * (1.0 - PRISM_FADE)),
            // Pushed outward: even spacing leaves uncorrelated lines at L/R correlation 0.54.
            pan: pan_slot.map(|slot| {
                let even = slot as f32 / (LINES - 1) as f32 * 2.0 - 1.0;
                even.signum() * even.abs().sqrt()
            }),
            random_ratio: [0.0; LINES],
            detune: [0.0; LINES],
            motion_rate: [0.0; LINES],
            diffuser_ms: [[0.0; LINES]; DIFFUSER_STEPS],
            diffuser_sign: [[1.0; LINES]; DIFFUSER_STEPS],
            tune_degree: std::array::from_fn(|i| degrees[i]),
        };
        for i in 0..LINES {
            dna.damp[i] = rng.bipolar();
            dna.detune[i] = rng.bipolar();
            dna.motion_rate[i] = 0.7 + 0.6 * rng.next();
            dna.prism_ratio[i] = PRISM_RATIOS[(rng.next() * PRISM_RATIOS.len() as f32) as usize];
            // One log-uniform draw per bin keeps every seed spread out; bin 0 pins the longest line to Size.
            let jitter = if log_bin[i] == 0 { 0.0 } else { rng.next() };
            dna.random_ratio[i] = 0.25f32.powf((log_bin[i] as f32 + jitter) / LINES as f32);
            for step in 0..DIFFUSER_STEPS {
                dna.diffuser_ms[step][i] =
                    (1.0 + 4.0 * rng.next()) * (1 << step) as f32 * DIFFUSER_MAX_MS / 20.0;
                dna.diffuser_sign[step][i] = if rng.next() < 0.5 { -1.0 } else { 1.0 };
            }
        }
        dna
    }

    /// Target length of each line in ms, before Motion, Scatter and Bloom.
    pub fn line_ms(&self, size_ms: f32, geometry: f32, diffuse: f32) -> [f32; LINES] {
        std::array::from_fn(|i| {
            let ratio = GRID[i] + (self.random_ratio[i] - GRID[i]) * geometry;
            // Grid lengths share a divisor and ring at high diffuse; diffuse = 0 stays on the grid.
            size_ms * ratio * (1.0 + diffuse * self.detune[i] * DETUNE)
        })
    }

    // shortcut: pentatonic only, add a Scale param when other scales are wanted
    /// Resonant notes of the Tune combs for a key (semitones above C).
    pub fn tune_hz(&self, key: u8) -> [f32; COMBS] {
        let base = 33 + (key as i32 + 3) % 12;
        self.tune_degree.map(|degree| {
            let midi = base + 12 * (degree / 5) as i32 + MINOR_PENTATONIC[degree % 5];
            440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
        })
    }
}

/// Three Givens stages with distinct strides, so every line reaches every other.
fn rotate(x: &mut [f32; LINES], c: f32, s: f32) {
    for stride in [1, 2, 4] {
        for i in 0..LINES {
            if i & stride == 0 {
                let (a, b) = (x[i], x[i + stride]);
                x[i] = c * a + s * b;
                x[i + stride] = c * b - s * a;
            }
        }
    }
}

fn pole(hz: f32, sr: f32) -> f32 {
    (-2.0 * PI * hz / sr).exp()
}

/// Tone's pole, 0 at 20 kHz so an open knob adds no loss per pass; long decays would hear even a little.
fn tone_pole(hz: f32, sr: f32) -> f32 {
    let open = pole(20_000.0, sr);
    ((pole(hz, sr) - open) / (1.0 - open)).max(0.0)
}

fn time_coef(ms: f32, sr: f32) -> f32 {
    (-1.0 / (ms * 0.001 * sr).max(1.0)).exp()
}

fn glide(x: &mut f32, target: f32, coef: f32) {
    *x = target + (*x - target) * coef;
}

/// Identity below the knee: plain tanh loses x²/3 per pass, which erodes a Freeze at g = 1.
fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= LIMIT_KNEE {
        x
    } else {
        x.signum() * (LIMIT_KNEE + (1.0 - LIMIT_KNEE) * ((a - LIMIT_KNEE) / (1.0 - LIMIT_KNEE)).tanh())
    }
}

/// Linear 0..1 fade.
struct Ramp {
    value: f32,
    step: f32,
}

impl Ramp {
    fn new(sr: f32, ms: f32) -> Self {
        Self {
            value: 0.0,
            step: 1.0 / (sr * ms / 1000.0).max(1.0),
        }
    }

    fn next(&mut self, on: bool) -> f32 {
        self.value = if on {
            (self.value + self.step).min(1.0)
        } else {
            (self.value - self.step).max(0.0)
        };
        self.value
    }
}

/// Fractional-delay ring buffer, power-of-two sized so wrapping is a mask.
struct DelayLine {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
}

impl DelayLine {
    fn new(len: usize) -> Self {
        let n = len.max(8).next_power_of_two();
        Self {
            buf: vec![0.0; n],
            mask: n - 1,
            write: 0,
        }
    }

    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
    }

    /// Largest delay the 4-point kernel can read without touching overwritten samples.
    fn max_delay(&self) -> f32 {
        (self.buf.len() - 3) as f32
    }

    /// 4-point Hermite read, `delay` samples behind the next write.
    fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(2.0, self.max_delay());
        let whole = d as usize;
        let t = d - whole as f32;
        let at = |back: usize| self.buf[(self.write + self.buf.len() - back) & self.mask];
        let (newer, a, b, older) = (at(whole - 1), at(whole), at(whole + 1), at(whole + 2));
        let c1 = 0.5 * (b - newer);
        let c2 = newer - 2.5 * a + 2.0 * b - 0.5 * older;
        let c3 = 0.5 * (older - newer) + 1.5 * (a - b);
        ((c3 * t + c2) * t + c1) * t + a
    }

    fn read_whole(&self, delay: usize) -> f32 {
        self.buf[(self.write + self.buf.len() - delay) & self.mask]
    }

    fn write(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) & self.mask;
    }
}

/// Two-tap delay-line pitch shifter; its gains sum to 1, so path gain never exceeds 1.
struct PitchShifter {
    line: DelayLine,
    phase: f32,
    ratio: f32,
    window: f32,
    /// Grain start offset per tap in samples, redrawn while that tap is silent.
    offset: [f32; 2],
    seed: u32,
    rng: Rng,
}

impl PitchShifter {
    fn new(sr: f32, ratio: f32, ms: f32, seed: u32) -> Self {
        Self {
            line: DelayLine::new((sr * REVERSE_MS * (1.0 + SHIFT_JITTER) * 0.001) as usize + 8),
            phase: 0.0,
            ratio,
            window: ms * 0.001 * sr,
            offset: [0.0; 2],
            seed,
            rng: Rng::new(seed),
        }
    }

    fn reset(&mut self) {
        self.line.reset();
        self.phase = 0.0;
        self.offset = [0.0; 2];
        self.rng = Rng::new(self.seed);
    }

    /// Reads after `line.write`; separate so a bypassed shifter keeps fresh audio without the reads.
    fn read(&mut self) -> f32 {
        let before = self.phase;
        self.phase = (self.phase + (1.0 - self.ratio) / self.window).rem_euclid(1.0);
        let taps = [self.phase, (self.phase + 0.5) % 1.0];
        // A fixed grain grid puts the output on lines 1/period apart, off the true pitch; random starts smear them onto it.
        for (k, was) in [before, (before + 0.5) % 1.0].into_iter().enumerate() {
            if (taps[k] - was).abs() > 0.5 {
                self.offset[k] = self.rng.next() * SHIFT_JITTER * self.window;
            }
        }
        // Each tap holds full gain between sin² crossfades, so less of the time is spent in the lossy overlap.
        let edge = (self.phase.min(1.0 - self.phase) - 0.25 + 0.25 * SHIFT_FADE) / (0.5 * SHIFT_FADE);
        let w = (0.5 * PI * edge.clamp(0.0, 1.0)).sin().powi(2);
        let read = |k: usize| self.line.read(taps[k] * self.window + self.offset[k]);
        read(0) * w + read(1) * (1.0 - w)
    }
}

/// Normalized comb `y[n] = (1 - g) x[n] + g y[n - T]`, peak gain 1 where a plain comb has 1/(1-g).
struct Comb {
    line: DelayLine,
    period: f32,
    g: f32,
    /// One-pole lowpass on the direct out, so the high harmonics don't spend its level budget.
    lp: f32,
    lp_z: f32,
}

impl Comb {
    fn tick(&mut self, x: f32) -> f32 {
        let y = (1.0 - self.g) * x + self.g * self.line.read(self.period);
        self.line.write(y);
        y
    }
}

#[derive(Clone, Copy)]
struct Head {
    base: f32,
    /// Scatter's read offset as a fraction of the length, so a jump can never pass the buffer.
    scale: f32,
}

struct Line {
    delay: DelayLine,
    heads: [Head; 2],
    active: usize,
    /// Weight of the outgoing head, 1 -> 0
    fade: f32,
    jump: bool,
    target: f32,
    t60: f32,
    g: f32,
    g_t: f32,
    lp: f32,
    lp_t: f32,
    lp_z: f32,
    hp: f32,
    hp_t: f32,
    hp_z: f32,
    p: f32,
    p_t: f32,
    shifter: PitchShifter,
    walk: f32,
    walk_t: f32,
    walk_phase: f32,
    walk_inc: f32,
    walk_coef: f32,
    pan: [f32; 2],
}

pub struct Engine {
    sr: f32,
    params: EngineParams,
    applied: (u32, u8),
    swap_pending: bool,
    fresh: bool,
    dna: SpaceDna,
    rng: Rng,
    lines: [Line; LINES],
    diffuser: [[DelayLine; LINES]; DIFFUSER_STEPS],
    diffuser_delay: [[usize; LINES]; DIFFUSER_STEPS],
    shimmer: [PitchShifter; 2],
    combs: [Comb; COMBS],
    rows: [[f32; LINES]; LINES],
    duck: Ramp,
    frz: Ramp,
    tick: u32,
    theta: f32,
    theta_t: f32,
    theta_coef: f32,
    c: f32,
    s: f32,
    wet_mix: f32,
    dry_mix: f32,
    comp: f32,
    comp_t: f32,
    a_s: f32,
    a_s_t: f32,
    a_t: f32,
    a_t_t: f32,
    width: f32,
    bloom: f32,
    env: f32,
    stretch: f32,
    motion_depth: f32,
    motion_depth_t: f32,
    scatter_p: f32,
    k_smooth: f32,
    k_len: f32,
    k_stretch: f32,
    k_attack: f32,
    k_release: f32,
    fade_step: f32,
    #[cfg(test)]
    peak_limit_in: f32,
    #[cfg(test)]
    peak_delay: f32,
}

impl Engine {
    /// Allocates every buffer at its maximum length. Everything below is allocation-free.
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let params = EngineParams::default();
        let line_len = (sr * max_delay_s()) as usize + 4;
        let line = |i: usize| Line {
            delay: DelayLine::new(line_len),
            heads: [Head { base: 1.0, scale: 1.0 }; 2],
            active: 0,
            fade: 0.0,
            jump: false,
            target: 1.0,
            t60: 1.0,
            g: 0.0,
            g_t: 0.0,
            lp: 0.0,
            lp_t: 0.0,
            lp_z: 0.0,
            hp: 0.0,
            hp_t: 0.0,
            hp_z: 0.0,
            p: 0.0,
            p_t: 0.0,
            shifter: PitchShifter::new(sr, 2.0, PRISM_MS, i as u32),
            walk: 0.0,
            walk_t: 0.0,
            walk_phase: 0.0,
            walk_inc: 0.0,
            walk_coef: 0.0,
            pan: [0.0; 2],
        };
        let mut engine = Self {
            sr,
            params,
            applied: (params.seed, params.key),
            swap_pending: false,
            fresh: true,
            dna: SpaceDna::from_seed(params.seed),
            rng: Rng::new(params.seed),
            lines: std::array::from_fn(line),
            diffuser: std::array::from_fn(|_| {
                std::array::from_fn(|_| DelayLine::new((sr * DIFFUSER_MAX_MS * 0.001) as usize * 2))
            }),
            diffuser_delay: [[1; LINES]; DIFFUSER_STEPS],
            shimmer: std::array::from_fn(|n| PitchShifter::new(sr, 2.0, PRISM_MS, (LINES + n) as u32)),
            combs: std::array::from_fn(|_| Comb {
                line: DelayLine::new((sr / TUNE_MIN_HZ) as usize + 8),
                period: 2.0,
                g: 0.0,
                lp: 0.0,
                lp_z: 0.0,
            }),
            rows: std::array::from_fn(|k| {
                std::array::from_fn(|i| {
                    let sign = if (k & i).count_ones() % 2 == 0 { 1.0 } else { -1.0 };
                    sign / (LINES as f32).sqrt()
                })
            }),
            duck: Ramp::new(sr, 25.0),
            frz: Ramp::new(sr, 20.0),
            tick: 0,
            theta: 0.0,
            theta_t: 0.0,
            theta_coef: time_coef(20.0 / CONTROL_PERIOD as f32, sr),
            c: 1.0,
            s: 0.0,
            wet_mix: 0.0,
            dry_mix: 1.0,
            comp: 1.0,
            comp_t: 1.0,
            a_s: 0.0,
            a_s_t: 0.0,
            a_t: 0.0,
            a_t_t: 0.0,
            width: 1.0,
            bloom: 0.0,
            env: 0.0,
            stretch: 1.0,
            motion_depth: 0.0,
            motion_depth_t: 0.0,
            scatter_p: 0.0,
            k_smooth: time_coef(10.0, sr),
            k_len: time_coef(150.0, sr),
            k_stretch: time_coef(150.0, sr),
            k_attack: time_coef(5.0, sr),
            k_release: time_coef(400.0, sr),
            fade_step: 1.0 / (HEAD_FADE_MS * 0.001 * sr),
            #[cfg(test)]
            peak_limit_in: 0.0,
            #[cfg(test)]
            peak_delay: 0.0,
        };
        engine.reset();
        engine
    }

    pub fn reset(&mut self) {
        for l in &mut self.lines {
            l.delay.reset();
            l.shifter.reset();
            l.lp_z = 0.0;
            l.hp_z = 0.0;
            l.walk = 0.0;
            l.walk_t = 0.0;
            l.walk_phase = 0.0;
        }
        for line in self.diffuser.iter_mut().flatten() {
            line.reset();
        }
        for shifter in &mut self.shimmer {
            shifter.reset();
        }
        self.duck.value = 1.0;
        self.frz.value = 0.0;
        self.tick = 0;
        self.env = 0.0;
        self.stretch = 1.0;
        self.fresh = true;
        #[cfg(test)]
        {
            self.peak_limit_in = 0.0;
            self.peak_delay = 0.0;
        }
        self.snap();
    }

    /// Call at least once per block, and every 32 samples while parameters move.
    pub fn set_params(&mut self, params: &EngineParams) {
        if *params == self.params {
            return;
        }
        self.params = *params;
        if self.fresh {
            self.snap();
        } else {
            // A knob that moves back before the duck bottoms out cancels the swap.
            self.swap_pending = (params.seed, params.key) != self.applied;
            self.update_targets();
        }
    }

    /// Nothing is ringing yet, so every smoother jumps and the first samples are already right.
    fn snap(&mut self) {
        self.apply_dna();
        for l in &mut self.lines {
            l.heads = [Head { base: l.target, scale: 1.0 }; 2];
            l.active = 0;
            l.fade = 0.0;
            l.jump = false;
            l.lp = l.lp_t;
            l.hp = l.hp_t;
            l.p = l.p_t;
        }
        self.a_s = self.a_s_t;
        self.a_t = self.a_t_t;
        self.theta = self.theta_t;
        self.motion_depth = self.motion_depth_t;
        self.width = self.params.width;
        self.bloom = self.params.bloom;
        self.control();
        for l in &mut self.lines {
            l.g = l.g_t;
        }
        self.comp = self.comp_t;
    }

    /// Runs only with the duck at 0, so nothing seeded inside the loop can step.
    fn apply_dna(&mut self) {
        let (seed, key) = (self.params.seed, self.params.key);
        self.applied = (seed, key);
        self.swap_pending = false;
        self.dna = SpaceDna::from_seed(seed);
        self.rng = Rng::new(seed ^ 0x5EED_5EED);
        for (i, l) in self.lines.iter_mut().enumerate() {
            l.jump = true;
            let angle = (self.dna.pan[i] + 1.0) * FRAC_PI_4;
            l.pan = [angle.cos(), angle.sin()];
            l.shifter.ratio = self.dna.prism_ratio[i];
            let ms = if l.shifter.ratio < 0.0 { REVERSE_MS } else { PRISM_MS };
            l.shifter.window = ms * 0.001 * self.sr;
        }
        for step in 0..DIFFUSER_STEPS {
            for i in 0..LINES {
                self.diffuser_delay[step][i] =
                    ((self.dna.diffuser_ms[step][i] * 0.001 * self.sr) as usize).max(1);
            }
        }
        for (comb, hz) in self.combs.iter_mut().zip(self.dna.tune_hz(key)) {
            comb.line.reset();
            comb.lp_z = 0.0;
            comb.lp = pole(TUNE_HARMONICS * hz, self.sr);
            comb.period = self.sr / hz;
        }
        self.update_targets();
    }

    fn update_targets(&mut self) {
        let p = self.params;
        let ms = self.dna.line_ms(p.size_ms, p.geometry, p.diffuse);
        let walk_hz = 0.05 + 2.95 * p.motion;
        for (i, l) in self.lines.iter_mut().enumerate() {
            let spread = p.material * self.dna.damp[i];
            l.target = ms[i] * 0.001 * self.sr;
            l.t60 = p.decay_s * 2f32.powf(-spread);
            l.lp_t = tone_pole(p.tone_hz * 2f32.powf(-2.0 * spread), self.sr);
            l.hp_t = pole(p.low_cut_hz * 2f32.powf(spread), self.sr);
            l.p_t = ((p.prism - self.dna.prism_entry[i]) / PRISM_FADE).clamp(0.0, 1.0);
            let hz = walk_hz * self.dna.motion_rate[i];
            l.walk_inc = hz / self.sr;
            l.walk_coef = pole(hz, self.sr);
        }
        let ring_s = p.decay_s.clamp(0.5, 4.0);
        for comb in &mut self.combs {
            let ring_s = ring_s.min(TUNE_PERIODS * comb.period / self.sr);
            comb.g = 10f32.powf(-3.0 * comb.period / self.sr / ring_s);
        }
        let over = (p.shimmer + p.tune * TUNE_MAX).max(1.0);
        self.a_s_t = p.shimmer / over;
        self.a_t_t = p.tune * TUNE_MAX / over;
        self.theta_t = p.diffuse * FRAC_PI_4;
        self.motion_depth_t = p.motion * p.motion * MOTION_S * self.sr;
        self.scatter_p = p.scatter * p.scatter * SCATTER_RATE / self.sr;
    }

    /// Length-dependent values, recomputed at control rate because Bloom and Scatter move lengths.
    fn control(&mut self) {
        let shimmer_rows = (1.0 - self.a_s_t).powi(2) + SHIFT_ENERGY * self.a_s_t * self.a_s_t;
        let tune_rows = (1.0 - self.a_t_t).powi(2);
        let pass = (SHIMMER_ROWS.len() as f32 * shimmer_rows + TUNE_ROWS.len() as f32 * tune_rows) / LINES as f32;
        let mut energy = 0.0;
        for l in &mut self.lines {
            let h = l.heads[l.active];
            // The shifter's average read delay lengthens the pass.
            let len_s = (h.base * h.scale * self.stretch + l.p_t * 0.5 * (1.0 + SHIFT_JITTER) * l.shifter.window) / self.sr;
            let prism = (1.0 - l.p_t).powi(2) + SHIFT_ENERGY * l.p_t * l.p_t;
            let g = 10f32.powf(-3.0 * len_s / l.t60);
            // Makes up what the sounds take per pass so Decay keeps its meaning; the cap stays below 1.
            let makeup = pass.powf(-0.5 * DECAY_MAKEUP) * prism.powf(-0.5 * PRISM_MAKEUP);
            l.g_t = (g * makeup).min(g.powf(1.0 / MAKEUP_RING));
            energy += l.g_t * l.g_t * pass * prism;
        }
        // On the input, not the wet: lines stay under the limiter and a ringing tail never jumps.
        self.comp_t = (1.0 - energy / LINES as f32).powf(0.5 * COMP_DEPTH);
        glide(&mut self.theta, self.theta_t, self.theta_coef);
        (self.s, self.c) = self.theta.sin_cos();
        // Equal power because the two paths are uncorrelated; 2θ runs 0..π/2 over the knob.
        (self.wet_mix, self.dry_mix) = (2.0 * self.theta).sin_cos();
    }

    /// Returns the 100% wet stereo signal; Mix and Out live in the wrapper.
    pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        if self.tick == 0 {
            self.control();
            self.tick = CONTROL_PERIOD;
        }
        self.tick -= 1;
        self.fresh = false;

        let duck = self.duck.next(!self.swap_pending);
        if self.swap_pending && duck == 0.0 {
            self.apply_dna();
        }
        let frz = self.frz.next(self.params.freeze);
        // A lossless hold: Prism and the reinjection would each bleed the frozen tail away in seconds.
        let thaw = 1.0 - frz;
        let freeze = self.params.freeze;
        let k = self.k_smooth;

        let level = input[0].abs().max(input[1].abs());
        let k_env = if level > self.env { self.k_attack } else { self.k_release };
        glide(&mut self.env, level, k_env);
        glide(&mut self.bloom, self.params.bloom, k);
        let bloom = self.bloom * (self.env * BLOOM_SENS).min(1.0);
        glide(&mut self.stretch, 1.0 + BLOOM_STRETCH * bloom, self.k_stretch);
        glide(&mut self.comp, self.comp_t, k);
        glide(&mut self.a_s, self.a_s_t, k);
        glide(&mut self.a_t, self.a_t_t, k);
        glide(&mut self.width, self.params.width, k);
        glide(&mut self.motion_depth, self.motion_depth_t, k);
        let (c, s) = (self.c, self.s);

        let gain = IN_SCALE * self.comp;
        let spread: [f32; LINES] = std::array::from_fn(|i| input[i % 2] * gain);
        let mut diffused = spread;
        for step in 0..DIFFUSER_STEPS {
            for i in 0..LINES {
                let line = &mut self.diffuser[step][i];
                let out = line.read_whole(self.diffuser_delay[step][i]);
                line.write(diffused[i]);
                // Ducked per step: a jump at step 0 would otherwise ride the later steps past the duck.
                diffused[i] = out * self.dna.diffuser_sign[step][i] * duck;
            }
            rotate(&mut diffused, c, s);
        }
        // The swap moves the diffuser and whatever enters here recirculates, so input ducks too.
        let enter = thaw * duck;
        let line_in: [f32; LINES] =
            std::array::from_fn(|i| (spread[i] * self.dry_mix + diffused[i] * self.wet_mix) * enter);

        let stretch = self.stretch;
        let depth = self.motion_depth;
        let mut x = [0.0f32; LINES];
        let mut out = [0.0f32; 2];
        for i in 0..LINES {
            let l = &mut self.lines[i];
            let active = l.active;
            // Only the live head glides, so the head fading out cannot chirp.
            glide(&mut l.heads[active].base, l.target, self.k_len);
            glide(&mut l.g, l.g_t, k);
            glide(&mut l.lp, l.lp_t, k);
            glide(&mut l.hp, l.hp_t, k);
            glide(&mut l.p, l.p_t, k);
            l.walk_phase += l.walk_inc;
            if l.walk_phase >= 1.0 {
                l.walk_phase -= 1.0;
                l.walk_t = self.rng.bipolar();
            }
            glide(&mut l.walk, l.walk_t, l.walk_coef);

            let live = l.heads[active].base * l.heads[active].scale * stretch;
            let live = live + l.walk * depth.min(0.5 * live);
            let mut v = l.delay.read(live);
            #[cfg(test)]
            {
                self.peak_delay = self.peak_delay.max(live);
            }
            if l.fade > 0.0 {
                let old = l.heads[1 - active].base * l.heads[1 - active].scale * stretch;
                let old = old + l.walk * depth.min(0.5 * old);
                v += (l.delay.read(old) - v) * l.fade;
                l.fade = (l.fade - self.fade_step).max(0.0);
            } else {
                let scatter = !freeze && self.scatter_p > 0.0 && (l.jump || self.rng.next() < self.scatter_p);
                let go_home = !freeze && self.scatter_p == 0.0 && l.heads[active].scale != 1.0;
                if l.jump || scatter || go_home {
                    let scale = if scatter {
                        SCATTER_MIN_SCALE + (1.0 - SCATTER_MIN_SCALE) * self.rng.next()
                    } else {
                        1.0
                    };
                    let base = if l.jump { l.target } else { l.heads[active].base };
                    l.heads[1 - active] = Head { base, scale };
                    l.active = 1 - active;
                    l.fade = 1.0;
                    l.jump = false;
                }
            }

            l.lp_z = v + (l.lp_z - v) * l.lp;
            l.hp_z = l.lp_z + (l.hp_z - l.lp_z) * l.hp;
            let filtered = l.lp_z - l.hp_z;
            let filtered = filtered + (v - filtered) * frz;

            l.shifter.line.write(filtered);
            let p = l.p * duck * thaw;
            let shaped = if p > 0.0 {
                filtered + (l.shifter.read() - filtered) * p
            } else {
                filtered
            };
            x[i] = shaped;
            out[0] += shaped * l.pan[0];
            out[1] += shaped * l.pan[1];
        }

        rotate(&mut x, c, s);

        // Written as Mx + a(inj - Mx): inj keeps the rest of Mx and swaps only the rows it taps,
        // so ||inj|| <= ||x|| holds and the other lines keep the tail alive.
        let (a_s, a_t) = (self.a_s * thaw, self.a_t * duck * thaw);
        let direct = self.a_t / TUNE_MAX * TUNE_OUT;
        let mut mixed = x;
        for (n, shifter) in self.shimmer.iter_mut().enumerate() {
            let row = &self.rows[SHIMMER_ROWS[n]];
            let tap: f32 = (0..LINES).map(|i| row[i] * x[i]).sum();
            shifter.line.write(tap);
            let delta = a_s * (shifter.read() - tap);
            for i in 0..LINES {
                mixed[i] += delta * row[i];
            }
        }
        for (n, comb) in self.combs.iter_mut().enumerate() {
            let row = &self.rows[TUNE_ROWS[n]];
            let tap: f32 = (0..LINES).map(|i| row[i] * x[i]).sum();
            let y = comb.tick(tap * duck);
            comb.lp_z = y + (comb.lp_z - y) * comb.lp;
            out[n % 2] += TUNE_CEILING * soft_limit(direct * comb.lp_z / TUNE_CEILING);
            let delta = a_t * (y - tap);
            for i in 0..LINES {
                mixed[i] += delta * row[i];
            }
        }

        for i in 0..LINES {
            let l = &mut self.lines[i];
            #[cfg(test)]
            {
                self.peak_limit_in = self.peak_limit_in.max(mixed[i].abs());
            }
            let g = l.g + (1.0 - l.g) * frz;
            l.delay.write(line_in[i] + g * soft_limit(mixed[i]));
        }

        let wet = OUT_GAIN * duck * (1.0 - BLOOM_DUCK * bloom);
        let mid = (out[0] + out[1]) * 0.5;
        let side = (out[0] - out[1]) * 0.5 * self.width;
        // Echo mode at long Decay resonates a steady sine to +16 dB; cap the wet at full scale
        [soft_limit((mid + side) * wet), soft_limit((mid - side) * wet)]
    }
}

fn max_delay_s() -> f32 {
    // A 50 ms margin on top of the worst stacked case: longest line, Bloom stretch and Motion.
    MAX_SIZE_S * (1.0 + DETUNE) * (1.0 + BLOOM_STRETCH) + MOTION_S + 0.05
}

#[cfg(test)]
mod tests;
