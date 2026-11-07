use super::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const SR: f32 = 48_000.0;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

// The wrapper's assert_process_allocs aborts on any allocation, so catch one here first.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn secs(s: f32) -> usize {
    (s * SR) as usize
}

/// Bare reverb: only Size, Decay and Seed shape the sound.
fn bare() -> EngineParams {
    EngineParams {
        diffuse: 1.0,
        tone_hz: 20_000.0,
        low_cut_hz: 20.0,
        material: 0.0,
        motion: 0.0,
        ..EngineParams::default()
    }
}

fn engine(p: &EngineParams) -> Engine {
    let mut e = Engine::new(SR);
    e.set_params(p);
    e
}

fn run(e: &mut Engine, n: usize, mut input: impl FnMut() -> [f32; 2]) -> Vec<[f32; 2]> {
    (0..n).map(|_| e.process(input())).collect()
}

fn silence() -> [f32; 2] {
    [0.0; 2]
}

fn rms(x: &[[f32; 2]]) -> f32 {
    let e: f64 = x.iter().map(|f| (f[0] as f64).powi(2) + (f[1] as f64).powi(2)).sum();
    ((e / (2 * x.len()) as f64).sqrt()) as f32
}

fn db(a: f32, b: f32) -> f32 {
    20.0 * (a / b).log10()
}

fn max_step(x: &[[f32; 2]]) -> f32 {
    x.windows(2)
        .map(|w| (w[1][0] - w[0][0]).abs().max((w[1][1] - w[0][1]).abs()))
        .fold(0.0, f32::max)
}

/// Hann-windowed single-bin magnitude of the mid channel over one frame; a full-scale sine reads 1.
fn goertzel_frame(x: &[[f32; 2]], hz: f32) -> f64 {
    let n = x.len() as f64;
    let w = 2.0 * std::f64::consts::PI * hz as f64 / SR as f64;
    let coef = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for (i, f) in x.iter().enumerate() {
        let hann = 1.0 - (2.0 * std::f64::consts::PI * i as f64 / n).cos();
        let s0 = hann * ((f[0] + f[1]) * 0.5) as f64 + coef * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coef * s1 * s2).max(0.0).sqrt() * 2.0 / n
}

/// RMS over 100 ms Goertzel frames. A delay-line shifter has lines spaced 1/period around the shifted
/// note and none exactly on it, so a long single-bin window would read between them.
fn goertzel(x: &[[f32; 2]], hz: f32) -> f32 {
    goertzel_frames(x, hz, 0.1)
}

fn goertzel_frames(x: &[[f32; 2]], hz: f32, frame_s: f32) -> f32 {
    let frames: Vec<f64> = x.chunks_exact(secs(frame_s)).map(|f| goertzel_frame(f, hz).powi(2)).collect();
    (frames.iter().sum::<f64>() / frames.len() as f64).sqrt() as f32
}

fn sine(hz: f32, amp: f32) -> impl FnMut() -> [f32; 2] {
    let mut phase = 0.0f64;
    move || {
        phase += hz as f64 / SR as f64;
        let v = amp * (2.0 * std::f64::consts::PI * phase).sin() as f32;
        [v, v]
    }
}

fn noise(amp: f32) -> impl FnMut() -> [f32; 2] {
    let mut rng = Rng::new(99);
    move || [amp * rng.bipolar(), amp * rng.bipolar()]
}

/// Hermite reads lose highs on every pass, so white noise loses RMS in Freeze even at loop gain 1.
fn lowpassed_noise(amp: f32) -> impl FnMut() -> [f32; 2] {
    let mut white = noise(amp);
    let coef = pole(2500.0, SR);
    let mut z = [[0.0f32; 2]; 2];
    move || {
        let w = white();
        std::array::from_fn(|ch| {
            z[0][ch] = w[ch] + (z[0][ch] - w[ch]) * coef;
            z[1][ch] = z[0][ch] + (z[1][ch] - z[0][ch]) * coef;
            z[1][ch]
        })
    }
}

/// T30 from Schroeder integration of a stereo impulse response, in seconds.
fn t30(ir: &[[f32; 2]]) -> f32 {
    let mut edc = vec![0.0f64; ir.len()];
    let mut acc = 0.0f64;
    for i in (0..ir.len()).rev() {
        acc += (ir[i][0] as f64).powi(2) + (ir[i][1] as f64).powi(2);
        edc[i] = acc;
    }
    let curve: Vec<(f64, f64)> = edc
        .iter()
        .enumerate()
        .map(|(i, e)| (i as f64 / SR as f64, 10.0 * (e / edc[0]).log10()))
        .filter(|(_, d)| (-35.0..=-5.0).contains(d))
        .collect();
    let n = curve.len() as f64;
    let (mt, md) = curve.iter().fold((0.0, 0.0), |a, c| (a.0 + c.0 / n, a.1 + c.1 / n));
    let cov: f64 = curve.iter().map(|c| (c.0 - mt) * (c.1 - md)).sum();
    let var: f64 = curve.iter().map(|c| (c.0 - mt).powi(2)).sum();
    (-60.0 / (cov / var)) as f32
}

#[test]
fn matrix_preserves_energy_at_all_angles() {
    let mut rng = Rng::new(1);
    for step in 0..=32 {
        let (s, c) = (step as f32 / 32.0 * FRAC_PI_4).sin_cos();
        let mut x: [f32; LINES] = std::array::from_fn(|_| rng.bipolar());
        let before: f32 = x.iter().map(|v| v * v).sum();
        rotate(&mut x, c, s);
        let after: f32 = x.iter().map(|v| v * v).sum();
        assert!((before - after).abs() < 1e-5, "step {step}: {before} vs {after}");
    }
}

#[test]
fn matrix_at_full_angle_is_hadamard_and_zero_is_identity() {
    let (s, c) = FRAC_PI_4.sin_cos();
    for col in 0..LINES {
        let mut x = [0.0; LINES];
        x[col] = 1.0;
        rotate(&mut x, c, s);
        for v in x {
            assert!((v.abs() - 1.0 / 8f32.sqrt()).abs() < 1e-6, "{v}");
        }
        let mut y = [0.0; LINES];
        y[col] = 1.0;
        rotate(&mut y, 1.0, 0.0);
        assert_eq!(y[col], 1.0);
    }
}

#[test]
fn hermite_never_amplifies() {
    // Freeze runs at g = 1, so any fractional read above unity would grow without bound.
    let mut worst = 0.0f32;
    for t in 0..=50 {
        let t = t as f32 / 50.0;
        let c = [
            -0.5 * t * t * t + t * t - 0.5 * t,
            1.5 * t * t * t - 2.5 * t * t + 1.0,
            -1.5 * t * t * t + 2.0 * t * t + 0.5 * t,
            0.5 * t * t * t - 0.5 * t * t,
        ];
        for bin in 0..=400 {
            let w = bin as f32 / 400.0 * PI;
            let (re, im) = c.iter().enumerate().fold((0.0, 0.0), |a, (k, v)| {
                (a.0 + v * (w * k as f32).cos(), a.1 - v * (w * k as f32).sin())
            });
            worst = worst.max((re * re + im * im).sqrt());
        }
    }
    eprintln!("hermite peak gain {worst}");
    assert!(worst <= 1.0 + 1e-4, "{worst}");
}

#[test]
fn diffuse_controls_echo_density() {
    let density = |diffuse: f32| {
        let p = EngineParams { diffuse, geometry: 0.0, ..bare() };
        let mut e = engine(&p);
        let mut first = true;
        let ir = run(&mut e, secs(1.0), || {
            let v = if first { 1.0 } else { 0.0 };
            first = false;
            [v, v]
        });
        let peak = ir.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        ir.iter().filter(|f| f[0].abs() > 0.01 * peak).count()
    };
    let (sparse, dense) = (density(0.0), density(1.0));
    eprintln!("samples over -40 dB in 1 s: diffuse 0 = {sparse}, diffuse 1 = {dense}");
    assert!(sparse < secs(1.0) / 50, "{sparse}");
    assert!(dense > sparse * 10, "{dense} vs {sparse}");
}

#[test]
fn echoes_land_on_the_grid_at_zero_diffuse() {
    let p = EngineParams { diffuse: 0.0, geometry: 0.0, size_ms: 500.0, decay_s: 6.0, ..bare() };
    let mut e = engine(&p);
    let mut first = true;
    let ir = run(&mut e, secs(0.6), || {
        let v = if first { 1.0 } else { 0.0 };
        first = false;
        [v, v]
    });
    let peak = ir.iter().map(|f| f[0].abs().max(f[1].abs())).fold(0.0, f32::max);
    // First arrival of every line sits on its grid length; nothing comes before the shortest.
    let shortest = GRID.iter().cloned().fold(1.0, f32::min) * 500.0;
    assert!(ir[..secs(shortest * 0.001) - 2].iter().all(|f| f[0].abs() < 1e-6 && f[1].abs() < 1e-6));
    for ratio in GRID {
        let at = secs(ratio * 0.5);
        let near = ir[at - 2..at + 3].iter().map(|f| f[0].abs().max(f[1].abs())).fold(0.0, f32::max);
        assert!(near > 0.2 * peak, "no echo at ratio {ratio}: {near} vs {peak}");
    }
}

#[test]
fn process_and_controls_do_not_allocate() {
    let p = EngineParams { scatter: 1.0, prism: 1.0, shimmer: 1.0, tune: 1.0, bloom: 1.0, motion: 1.0, ..EngineParams::default() };
    let mut e = Engine::new(SR);
    let mut src = noise(0.5);
    let before = ALLOCATIONS.with(Cell::get);
    e.set_params(&p);
    for i in 0..secs(1.0) {
        if i == secs(0.5) {
            e.set_params(&EngineParams { seed: 77, key: 3, freeze: true, ..p });
        }
        e.process(src());
    }
    e.reset();
    e.process(src());
    assert_eq!(ALLOCATIONS.with(Cell::get), before);
}

#[test]
fn same_seed_after_reset_is_bit_identical_and_seeds_differ() {
    let p = EngineParams { scatter: 0.5, motion: 0.6, prism: 0.6, shimmer: 0.3, tune: 0.3, bloom: 0.4, ..EngineParams::default() };
    let mut e = engine(&p);
    let a = run(&mut e, secs(2.0), noise(0.3));
    e.reset();
    e.set_params(&p);
    let b = run(&mut e, secs(2.0), noise(0.3));
    assert!(a.iter().zip(&b).all(|(x, y)| x[0].to_bits() == y[0].to_bits() && x[1].to_bits() == y[1].to_bits()));

    let mut other = engine(&EngineParams { seed: 4321, ..p });
    let c = run(&mut other, secs(2.0), noise(0.3));
    assert!(a.iter().zip(&c).any(|(x, y)| x != y));
}

#[test]
fn seed_zero_has_distinct_lengths() {
    let ms = SpaceDna::from_seed(0).line_ms(100.0, 1.0, 1.0);
    for i in 0..LINES {
        for j in 0..i {
            assert!((ms[i] - ms[j]).abs() > 0.1, "{ms:?}");
        }
    }
    // Bin 0 pins the longest line to Size before detune, for every seed.
    for seed in [0, 1, 42, 9999] {
        let plain = SpaceDna::from_seed(seed).line_ms(100.0, 1.0, 0.0);
        assert_eq!(plain.iter().cloned().fold(0.0, f32::max), 100.0);
    }
}

#[test]
fn diffuse_zero_stays_on_the_grid() {
    let ms = SpaceDna::from_seed(7).line_ms(100.0, 0.0, 0.0);
    for i in 0..LINES {
        assert_eq!(ms[i], GRID[i] * 100.0);
    }
}

#[test]
fn line_lengths_follow_ms_at_any_sample_rate() {
    let p = EngineParams::default();
    let ms = SpaceDna::from_seed(p.seed).line_ms(p.size_ms, p.geometry, p.diffuse);
    for sr in [44_100.0, 96_000.0] {
        let mut e = Engine::new(sr);
        e.set_params(&p);
        for i in 0..LINES {
            assert!((e.lines[i].target - ms[i] * 0.001 * sr).abs() < 1.0);
            let h = e.lines[i].heads[e.lines[i].active];
            assert!((h.base - ms[i] * 0.001 * sr).abs() < 1.0);
        }
    }
}

#[test]
fn largest_target_fits_the_buffer() {
    for sr in [44_100.0f32, 96_000.0, 192_000.0] {
        let p = EngineParams {
            size_ms: 2000.0,
            diffuse: 1.0,
            geometry: 1.0,
            motion: 1.0,
            bloom: 1.0,
            ..EngineParams::default()
        };
        let mut e = Engine::new(sr);
        e.set_params(&p);
        let capacity = e.lines[0].delay.max_delay();
        for seed in 0..=9999 {
            let longest = SpaceDna::from_seed(seed)
                .line_ms(p.size_ms, p.geometry, p.diffuse)
                .iter()
                .cloned()
                .fold(0.0, f32::max);
            let worst = longest * 0.001 * sr * (1.0 + BLOOM_STRETCH) + MOTION_S * sr;
            assert!(worst < capacity, "seed {seed} at {sr}: {worst} >= {capacity}");
        }
        // The real delay path under a loud input, for long enough that the stretch settles.
        let mut src = sine(100.0, 1.0);
        for _ in 0..(sr * 2.0) as usize {
            e.process(src());
        }
        eprintln!("sr {sr}: peak delay {} of {capacity}", e.peak_delay);
        assert!(e.peak_delay < capacity);
        assert!(e.peak_delay > 1.4 * 2000.0 * 0.001 * sr, "bloom never stretched: {}", e.peak_delay);
    }
}

#[test]
fn silence_in_silence_out() {
    let mut e = engine(&EngineParams::default());
    assert!(run(&mut e, secs(1.0), silence).iter().all(|f| f[0] == 0.0 && f[1] == 0.0));
}

/// Worst step around the swap over the steady-state step of either seed: gain at a fixed frequency
/// differs by many dB between seeds, and a low sine leaves about 38x of margin to a real click.
fn seed_change_ratio(p: EngineParams, to_seed: u32) -> f32 {
    let mut e = engine(&p);
    let mut src = sine(150.0, 0.5);
    run(&mut e, secs(p.decay_s * 2.0), &mut src);
    let before = max_step(&run(&mut e, secs(1.0), &mut src));
    e.set_params(&EngineParams { seed: to_seed, ..p });
    let around = max_step(&run(&mut e, secs(1.5), &mut src));
    run(&mut e, secs(p.decay_s * 2.0), &mut src);
    let settled = max_step(&run(&mut e, secs(1.0), &mut src));
    around / before.max(settled)
}

#[test]
fn seed_change_does_not_click() {
    for (from, to) in [(1234, 77), (77, 4321), (5, 0), (9999, 1)] {
        let default = EngineParams { seed: from, ..EngineParams::default() };
        let ratio = seed_change_ratio(default, to);
        // Short lines recirculate many times inside the measured 1.5 s.
        let small = seed_change_ratio(EngineParams { size_ms: 40.0, ..default }, to);
        let features = seed_change_ratio(
            EngineParams { prism: 0.7, tune: 0.4, material: 1.0, scatter: 0.5, ..default },
            to,
        );
        eprintln!("seed {from}->{to} step ratio: default {ratio:.2}, small {small:.2}, features {features:.2}");
        assert!(ratio < 3.0, "{ratio}");
        assert!(small < 3.0, "{small}");
        assert!(features < 3.0, "{features}");
    }
}

#[test]
fn rt60_matches_decay() {
    for decay_s in [0.5, 1.0, 3.0, 10.0, 30.0] {
        let p = EngineParams { decay_s, ..bare() };
        let mut e = engine(&p);
        let mut first = true;
        let ir = run(&mut e, secs((decay_s * 1.2).max(2.0)), || {
            let v = if first { 1.0 } else { 0.0 };
            first = false;
            [v, v]
        });
        let measured = t30(&ir);
        eprintln!("decay {decay_s}: measured RT60 {measured:.3} ({:+.1}%)", (measured / decay_s - 1.0) * 100.0);
        assert!((measured / decay_s - 1.0).abs() < 0.2, "{decay_s}: {measured}");
    }
}

/// The render example's woodblock hit.
fn click() -> impl FnMut() -> [f32; 2] {
    let mut t = 0.0f32;
    move || {
        let v = if t < 0.03 { 0.8 * (-t / 0.006).exp() * (std::f32::consts::TAU * 900.0 * t).sin() } else { 0.0 };
        t += 1.0 / SR;
        [v, v]
    }
}

fn spread_db(levels: &[f32]) -> f32 {
    let (lo, hi) = levels.iter().fold((f32::MAX, 0.0f32), |a, &v| (a.0.min(v), a.1.max(v)));
    db(hi, lo)
}

#[test]
fn sustained_level_stays_close_across_decay() {
    let levels: Vec<f32> = [0.1, 0.5, 2.0, 8.0, 30.0]
        .iter()
        .map(|&decay_s: &f32| {
            let mut e = engine(&EngineParams { decay_s, ..bare() });
            let mut src = noise(0.3);
            run(&mut e, secs((decay_s / 3.0).max(1.0)), &mut src);
            rms(&run(&mut e, secs(1.0), &mut src))
        })
        .collect();
    eprintln!("sustained wet RMS {levels:?}, spread {:.2} dB", spread_db(&levels));
    // Looser than full flattening on purpose, so hits still come through at long Decay.
    assert!(spread_db(&levels) <= 9.0);
}

#[test]
fn clicks_keep_their_peak_across_decay() {
    let peaks: Vec<f32> = [0.5, 3.0, 10.0, 30.0]
        .iter()
        .map(|&decay_s| {
            let mut e = engine(&EngineParams { decay_s, ..bare() });
            run(&mut e, secs(1.0), click()).iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()))
        })
        .collect();
    eprintln!("click wet peak {:?} dB, spread {:.2} dB", peaks.iter().map(|&p| 20.0 * p.log10()).collect::<Vec<_>>(), spread_db(&peaks));
    assert!(spread_db(&peaks) <= 7.0);
}

#[test]
fn freeze_holds_level() {
    let sounds = EngineParams { prism: 0.5, shimmer: 0.5, tune: 0.5, ..EngineParams::default() };
    for p in [EngineParams::default(), bare(), sounds] {
        // Loud on purpose: a Freeze that only holds at low level still ends up quiet on real input.
        let mut e = engine(&p);
        let mut src = lowpassed_noise(1.0);
        run(&mut e, secs(3.0), &mut src);
        e.set_params(&EngineParams { freeze: true, ..p });
        run(&mut e, secs(0.8), &mut src);
        let at_1 = rms(&run(&mut e, secs(0.4), &mut src));
        run(&mut e, secs(8.8), &mut src);
        let at_10 = rms(&run(&mut e, secs(0.4), &mut src));
        let drift = db(at_10, at_1);
        eprintln!("freeze RMS {at_1} -> {at_10}, drift {drift:+.2} dB, peak limiter input {}", e.peak_limit_in);
        assert!(at_1 > 0.01, "{at_1}");
        assert!(drift.abs() < 2.0, "{drift}");
    }
}

#[test]
fn long_decay_with_reinjection_stays_on_budget() {
    let p = EngineParams { decay_s: 30.0, shimmer: 1.0, tune: 1.0, ..EngineParams::default() };
    let mut e = engine(&p);
    run(&mut e, secs(3.0), noise(1.0));
    let peak = e.peak_limit_in;
    run(&mut e, secs(1.8), silence);
    let at_2 = rms(&run(&mut e, secs(0.4), silence));
    run(&mut e, secs(17.6), silence);
    let at_20 = rms(&run(&mut e, secs(0.4), silence));
    eprintln!("budget: RMS 2 s {at_2}, 20 s {at_20}, peak limiter input {peak} (full-scale noise), overall {}", e.peak_limit_in);
    assert!(at_20 < at_2);
    assert!(e.peak_limit_in < 1.0, "{}", e.peak_limit_in);
}

#[test]
fn sounds_keep_the_sustained_level() {
    let base = EngineParams { decay_s: 10.0, ..EngineParams::default() };
    let level = |p: EngineParams| {
        let mut e = engine(&p);
        let mut src = noise(0.3);
        run(&mut e, secs(4.0), &mut src);
        rms(&run(&mut e, secs(1.0), &mut src))
    };
    let dry = level(base);
    for (name, p) in [
        ("shimmer", EngineParams { shimmer: 1.0, ..base }),
        ("tune", EngineParams { tune: 1.0, ..base }),
        ("prism", EngineParams { prism: 1.0, ..base }),
        ("scatter", EngineParams { scatter: 1.0, ..base }),
    ] {
        let d = db(level(p), dry);
        eprintln!("sustained level with {name} at max: {d:+.1} dB");
        assert!(d.abs() < 6.0, "{name}: {d}");
    }
}

#[test]
fn everything_at_max_stays_finite() {
    let p = EngineParams {
        size_ms: 2000.0,
        decay_s: 30.0,
        diffuse: 1.0,
        tone_hz: 20_000.0,
        low_cut_hz: 1000.0,
        width: 1.0,
        freeze: false,
        seed: 9999,
        geometry: 1.0,
        material: 1.0,
        motion: 1.0,
        scatter: 1.0,
        prism: 1.0,
        shimmer: 1.0,
        tune: 1.0,
        key: 11,
        bloom: 1.0,
    };
    let mut e = engine(&p);
    let mut src = noise(1.0);
    assert!(run(&mut e, secs(30.0), &mut src).iter().all(|f| f[0].is_finite() && f[1].is_finite()));
    e.set_params(&EngineParams { freeze: true, ..p });
    assert!(run(&mut e, secs(30.0), &mut src).iter().all(|f| f[0].is_finite() && f[1].is_finite()));
    assert!(e.peak_limit_in.is_finite());
}

#[test]
fn resonant_echo_stays_under_full_scale() {
    let p = EngineParams { diffuse: 0.0, decay_s: 30.0, ..EngineParams::default() };
    let mut e = engine(&p);
    let out = run(&mut e, secs(20.0), sine(440.0, 1.0));
    let peak = out.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    assert!(peak <= 1.0, "peak = {peak}");
}

#[test]
fn shimmer_adds_the_octave() {
    let hz = 440.0;
    let level = |shimmer: f32| {
        let mut e = engine(&EngineParams { shimmer, ..bare() });
        let mut src = sine(hz, 0.5);
        run(&mut e, secs(2.0), &mut src);
        goertzel(&run(&mut e, secs(2.0), &mut src), 2.0 * hz)
    };
    let (off, on) = (level(0.0), level(0.6));
    eprintln!("shimmer at 2f: off {off:.2e}, on {on:.2e}, {:+.1} dB", db(on, off));
    assert!(db(on, off) > 20.0);
}

#[test]
fn prism_adds_the_chosen_interval() {
    let hz = 440.0;
    let fifth = 1.498_307;
    let seed = (0..)
        .find(|&s| SpaceDna::from_seed(s).prism_ratio.contains(&fifth))
        .unwrap();
    let level = |prism: f32| {
        let mut e = engine(&EngineParams { prism, seed, ..bare() });
        let mut src = sine(hz, 0.5);
        run(&mut e, secs(2.0), &mut src);
        goertzel(&run(&mut e, secs(2.0), &mut src), hz * fifth)
    };
    let (off, on) = (level(0.0), level(1.0));
    eprintln!("prism at +7 (seed {seed}): off {off:.2e}, on {on:.2e}, {:+.1} dB", db(on, off));
    assert!(db(on, off) > 20.0);
}

#[test]
fn prism_lines_enter_one_at_a_time() {
    let dna = SpaceDna::from_seed(1234);
    let active = |knob: f32| dna.prism_entry.iter().filter(|&&e| knob > e).count();
    assert_eq!(active(0.0), 0);
    assert_eq!(active(1.0), LINES);
    let mut sorted = dna.prism_entry;
    sorted.sort_by(f32::total_cmp);
    for w in sorted.windows(2) {
        assert!(w[1] > w[0]);
    }
}

#[test]
fn tune_notes_are_in_key_and_range() {
    for seed in [0, 1234, 9999] {
        let dna = SpaceDna::from_seed(seed);
        for key in 0..12u8 {
            for hz in dna.tune_hz(key) {
                let midi = 69.0 + 12.0 * (hz / 440.0).log2();
                let degree = (midi.round() as i32 - key as i32).rem_euclid(12);
                assert!(MINOR_PENTATONIC.contains(&degree), "{hz} in key {key}");
                assert!((54.0..=880.0).contains(&hz), "{hz}");
            }
        }
    }
}

#[test]
fn tune_raises_energy_at_its_notes() {
    // Noise, because a sine lands on whatever FDN mode is near it and reads many dB either way.
    let bin = |x: &[[f32; 2]], hz: f32| goertzel_frames(x, hz, 2.0).powi(2);
    let around = |x: &[[f32; 2]], hz: f32| {
        [-15.0, -12.0, -9.0, -6.0, -3.0, 3.0, 6.0, 9.0, 12.0, 15.0].iter().map(|d| bin(x, hz + d)).sum::<f32>() / 10.0
    };
    let mut gains = Vec::new();
    let mut prominence = Vec::new();
    let mut prominence_off = Vec::new();
    for seed in [1234, 7] {
        let p = EngineParams { decay_s: 6.0, seed, ..bare() };
        let render = |tune: f32| {
            let mut e = engine(&EngineParams { tune, ..p });
            let mut src = noise(0.3);
            run(&mut e, secs(3.0), &mut src);
            run(&mut e, secs(40.0), &mut src)
        };
        let (off, on) = (render(0.0), render(0.7));
        for hz in SpaceDna::from_seed(seed).tune_hz(p.key) {
            let gain = 10.0 * (bin(&on, hz) / bin(&off, hz)).log10();
            let peak_on = 10.0 * (bin(&on, hz) / around(&on, hz)).log10();
            let peak_off = 10.0 * (bin(&off, hz) / around(&off, hz)).log10();
            eprintln!("tune seed {seed} {hz:.1} Hz: {gain:+.1} dB vs off; over nearby bins {peak_off:+.1} -> {peak_on:+.1} dB");
            gains.push(gain);
            prominence.push(peak_on);
            prominence_off.push(peak_off);
        }
    }
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    eprintln!(
        "tune mean: {:+.1} dB vs off, over nearby bins {:+.1} -> {:+.1} dB",
        mean(&gains),
        mean(&prominence_off),
        mean(&prominence)
    );
    // Not per-note 3 dB: a note on a strong Tune = 0 mode starts high, but it must still not lose.
    assert!(gains.iter().all(|&d| d > 0.0), "{gains:?}");
    assert!(mean(&gains) > 6.0, "{gains:?}");
    assert!(mean(&prominence) > mean(&prominence_off) + 3.0);
}

#[test]
fn bloom_ducks_during_input_and_swells_after() {
    let hz = 220.0;
    let render = |bloom: f32| {
        let mut e = engine(&EngineParams { bloom, ..EngineParams::default() });
        let mut src = sine(hz, 0.5);
        run(&mut e, secs(1.5), &mut src);
        let during = run(&mut e, secs(0.5), &mut src);
        let after = run(&mut e, secs(1.2), silence);
        (goertzel(&during, hz), rms(&during), rms(&after[secs(0.4)..secs(0.8)]))
    };
    let (off_bin, off_during, off_after) = render(0.0);
    let (on_bin, on_during, on_after) = render(1.0);
    eprintln!(
        "bloom at f: off {off_bin:.3}, on {on_bin:.3}, {:+.1} dB; after/during off {:+.1} dB, on {:+.1} dB",
        db(on_bin, off_bin),
        db(off_after, off_during),
        db(on_after, on_during)
    );
    assert!(db(on_bin, off_bin) < -6.0);
    assert!(db(on_after, on_during) > db(off_after, off_during) + 6.0);
}

#[test]
fn scatter_changes_the_output_without_a_click() {
    let render = |scatter: f32| {
        let mut e = engine(&EngineParams { scatter, ..EngineParams::default() });
        let mut src = sine(150.0, 0.5);
        run(&mut e, secs(3.0), &mut src);
        run(&mut e, secs(3.0), &mut src)
    };
    let (off, on) = (render(0.0), render(1.0));
    let crest = |x: &[[f32; 2]]| max_step(x) / rms(x);
    let diff: Vec<[f32; 2]> = off.iter().zip(&on).map(|(x, y)| [x[0] - y[0], x[1] - y[1]]).collect();
    eprintln!(
        "scatter step/RMS off {:.4}, on {:.4}; RMS off {:.3}, on {:.3}; diff RMS {:+.1} dB",
        crest(&off),
        crest(&on),
        rms(&off),
        rms(&on),
        db(rms(&diff), rms(&off))
    );
    assert!(rms(&diff) > 0.2 * rms(&off));
    assert!(crest(&on) / crest(&off) < 3.0, "{}", crest(&on) / crest(&off));
}

#[test]
fn freeze_stops_scatter() {
    let p = EngineParams { scatter: 1.0, freeze: true, ..EngineParams::default() };
    let mut e = engine(&p);
    run(&mut e, secs(0.1), noise(0.3));
    let heads: [(usize, u32); LINES] =
        std::array::from_fn(|i| (e.lines[i].active, e.lines[i].heads[e.lines[i].active].scale.to_bits()));
    run(&mut e, secs(1.0), noise(0.3));
    for i in 0..LINES {
        assert_eq!(heads[i], (e.lines[i].active, e.lines[i].heads[e.lines[i].active].scale.to_bits()));
    }
}

#[test]
fn sounds_keep_the_decay() {
    let tail = |p: EngineParams| {
        let mut e = engine(&p);
        let mut first = true;
        let ir = run(&mut e, secs(p.decay_s * 1.2), || {
            let v = if first { 1.0 } else { 0.0 };
            first = false;
            [v, v]
        });
        t30(&ir)
    };
    for decay_s in [3.0, 10.0] {
        let base = EngineParams { decay_s, ..EngineParams::default() };
        let plain = tail(base);
        for (name, p) in [
            ("shimmer", EngineParams { shimmer: 0.5, ..base }),
            ("tune", EngineParams { tune: 0.5, ..base }),
            ("prism", EngineParams { prism: 0.5, ..base }),
            ("scatter", EngineParams { scatter: 0.5, ..base }),
        ] {
            let ratio = tail(p) / plain;
            eprintln!("decay {decay_s}: T30 plain {plain:.2} s, {name} at 0.5 keeps {:.0}%", ratio * 100.0);
            assert!(ratio >= 0.7, "{decay_s} {name}: {ratio}");
        }
    }
}

/// Energy of a shifted sine within ±3 Hz of the target pitch over the rest of ±60 Hz, in dB.
fn shifter_purity(hz: f32) -> f32 {
    let mut s = PitchShifter::new(SR, 2.0, PRISM_MS, 0);
    let mut src = sine(hz, 1.0);
    let out: Vec<[f32; 2]> = (0..secs(4.5))
        .map(|_| {
            s.line.write(src()[0]);
            let y = s.read();
            [y, y]
        })
        .collect();
    let (mut on, mut off) = (0.0, 0.0);
    for k in 0..=480 {
        let d = -60.0 + k as f32 * 0.25;
        let e = goertzel_frame(&out[secs(0.5)..], 2.0 * hz + d).powi(2);
        if d.abs() <= 3.0 {
            on += e;
        } else {
            off += e;
        }
    }
    10.0 * (on / off).log10() as f32
}

#[test]
fn shifter_lands_on_the_shifted_pitch() {
    // Several inputs, because a fixed grain grid lands a line on 2f only for lucky frequencies.
    let purity: Vec<f32> = (0..9).map(|k| shifter_purity(400.0 + 10.0 * k as f32)).collect();
    eprintln!("shifter energy within ±3 Hz of 2f over the rest of ±60 Hz, 400..480 Hz in: {purity:.1?} dB");
    assert!(purity.iter().all(|&p| p > -10.0), "{purity:?}");
}

#[test]
fn reverse_shifter_plays_backward() {
    // A rising sawtooth played backward falls slowly and jumps up.
    for (ratio, ms) in [(-1.0, REVERSE_MS), (2.0, PRISM_MS)] {
        let mut s = PitchShifter::new(SR, ratio, ms, 0);
        let mut t = 0.0f32;
        let y: Vec<f32> = (0..secs(2.0))
            .map(|_| {
                t = (t + 50.0 / SR) % 1.0;
                s.line.write(2.0 * t - 1.0);
                s.read()
            })
            .collect();
        let jumps = |up: bool| y.windows(2).filter(|w| (w[1] - w[0]) * if up { 1.0 } else { -1.0 } > 0.5).count();
        eprintln!("ratio {ratio}: {} jumps up, {} down", jumps(true), jumps(false));
        let (forward, backward) = if ratio < 0.0 { (jumps(false), jumps(true)) } else { (jumps(true), jumps(false)) };
        assert!(backward > 50 && forward == 0, "{ratio}: {backward} vs {forward}");
    }
}

#[test]
fn tune_catches_detuned_sources() {
    let mut gains = Vec::new();
    for seed in [1234, 7] {
        let p = EngineParams { decay_s: 6.0, seed, ..bare() };
        for hz in SpaceDna::from_seed(seed).tune_hz(p.key) {
            for cents in [-20.0f32, 20.0] {
                let src_hz = hz * 2f32.powf(cents / 1200.0);
                let level = |tune: f32| {
                    let mut e = engine(&EngineParams { tune, ..p });
                    let mut src = sine(src_hz, 0.1);
                    run(&mut e, secs(3.0), &mut src);
                    goertzel_frames(&run(&mut e, secs(4.0), &mut src), src_hz, 2.0)
                };
                gains.push(db(level(0.7), level(0.0)));
            }
        }
    }
    let mean = gains.iter().sum::<f32>() / gains.len() as f32;
    // The mean, because a sine on or between FDN modes reads many dB either way.
    eprintln!("tune at ±20 cents: mean {mean:+.1} dB vs off, {gains:.1?}");
    assert!(mean >= 6.0, "{gains:?}");
}
