//! Renders demo WAVs from synthesized inputs, and prints levels plus a realtime CPU ratio.
use seedspace_dsp::{Engine, EngineParams, SpaceDna};
use std::time::Instant;

const SR: f32 = 48_000.0;
const SECONDS: f32 = 12.0;
const DRY: f32 = 0.6;
const WET: f32 = 0.6;

/// Name, settings, input, and when Freeze engages.
type Demo<'a> = (&'a str, EngineParams, &'a [[f32; 2]], Option<f32>);

/// Woodblock-like hits every two seconds; a one-sample click would mostly vanish into Tone.
fn clicks() -> Vec<[f32; 2]> {
    let n = (SR * SECONDS) as usize;
    let mut out = vec![[0.0; 2]; n];
    for start in (0..n).step_by((SR * 2.0) as usize) {
        for i in 0..(SR * 0.03) as usize {
            let t = i as f32 / SR;
            let v = 0.8 * (-t / 0.006).exp() * (std::f32::consts::TAU * 900.0 * t).sin();
            out[start + i] = [v, v];
        }
    }
    out
}

/// One second of a minor seventh every three seconds, with silence between for the tail.
fn chord() -> Vec<[f32; 2]> {
    let n = (SR * SECONDS) as usize;
    let notes = [130.81f32, 155.56, 196.0, 233.08];
    let mut phase = [0.0f32; 8];
    (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let pos = t % 3.0;
            let env = (pos / 0.01).min(1.0) * ((1.0 - pos) / 0.05).clamp(0.0, 1.0);
            let mut frame = [0.0; 2];
            for (k, hz) in notes.iter().enumerate() {
                for ch in 0..2 {
                    // Detuned per side so the input itself is stereo.
                    let p = &mut phase[k * 2 + ch];
                    *p = (*p + hz * (1.0 + ch as f32 * 0.002) / SR) % 1.0;
                    frame[ch] += (2.0 * *p - 1.0) * 0.15 * env;
                }
            }
            frame
        })
        .collect()
}

fn render(params: &EngineParams, input: &[[f32; 2]], freeze_at_s: Option<f32>) -> Vec<[f32; 2]> {
    let mut engine = Engine::new(SR);
    engine.set_params(params);
    let freeze_at = freeze_at_s.map_or(usize::MAX, |s| (s * SR) as usize);
    input
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            if i == freeze_at {
                engine.set_params(&EngineParams { freeze: true, ..*params });
            }
            let w = engine.process(x);
            [x[0] * DRY + w[0] * WET, x[1] * DRY + w[1] * WET]
        })
        .collect()
}

fn write_wav(path: &std::path::Path, frames: &[[f32; 2]]) -> std::io::Result<()> {
    let data_len = (frames.len() * 4) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&(SR as u32).to_le_bytes());
    bytes.extend_from_slice(&(SR as u32 * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for frame in frames {
        for v in frame {
            bytes.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    std::fs::write(path, bytes)
}

fn main() -> std::io::Result<()> {
    let outdir = std::env::args().nth(1).unwrap_or_else(|| "demos".to_string());
    let outdir = std::path::Path::new(&outdir);
    std::fs::create_dir_all(outdir)?;

    let (clicks, chord) = (clicks(), chord());
    let base = EngineParams::default();
    let echo = EngineParams { size_ms: 600.0, decay_s: 6.0, diffuse: 0.0, geometry: 0.0, material: 0.0, motion: 0.0, ..base };
    let reverse_seed = (0..10_000)
        .max_by_key(|&s| SpaceDna::from_seed(s).prism_ratio.iter().filter(|&&r| r < 0.0).count())
        .unwrap();
    let demos: Vec<Demo> = vec![
        ("01_default_chord", base, &chord, None),
        ("02_default_clicks", base, &clicks, None),
        ("03_echo_polyrhythm", echo, &clicks, None),
        ("04_halfway_diffuse", EngineParams { diffuse: 0.5, ..echo }, &clicks, None),
        ("05_seed_a", EngineParams { seed: 11, decay_s: 5.0, ..base }, &chord, None),
        ("06_seed_b", EngineParams { seed: 2222, decay_s: 5.0, ..base }, &chord, None),
        ("07_seed_c", EngineParams { seed: 7777, decay_s: 5.0, ..base }, &chord, None),
        ("08_prism", EngineParams { prism: 0.7, decay_s: 5.0, ..base }, &chord, None),
        ("09_prism_reverse", EngineParams { prism: 1.0, decay_s: 5.0, seed: reverse_seed, ..base }, &chord, None),
        ("10_shimmer", EngineParams { shimmer: 0.7, decay_s: 8.0, ..base }, &chord, None),
        ("11_scatter", EngineParams { scatter: 0.6, size_ms: 400.0, ..base }, &clicks, None),
        ("12_tune_clicks", EngineParams { tune: 0.8, decay_s: 6.0, ..base }, &clicks, None),
        ("13_tune_chord", EngineParams { tune: 0.8, decay_s: 6.0, ..base }, &chord, None),
        ("14_bloom", EngineParams { bloom: 1.0, decay_s: 5.0, ..base }, &chord, None),
        ("15_freeze", EngineParams { decay_s: 6.0, ..base }, &chord, Some(0.8)),
    ];
    println!("reverse-heavy seed: {reverse_seed}");
    for (name, params, input, freeze_at) in demos {
        let frames = render(&params, input, freeze_at);
        let peak = frames.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs()));
        let rms = (frames.iter().flatten().map(|v| v * v).sum::<f32>() / (frames.len() * 2) as f32).sqrt();
        write_wav(&outdir.join(format!("{name}.wav")), &frames)?;
        println!("{name:<22} peak {peak:6.3}  rms {rms:6.3}");
    }

    let all_on = EngineParams {
        size_ms: 1500.0,
        decay_s: 20.0,
        diffuse: 1.0,
        motion: 0.8,
        scatter: 1.0,
        prism: 1.0,
        shimmer: 1.0,
        tune: 1.0,
        bloom: 1.0,
        ..base
    };
    let mut engine = Engine::new(SR);
    engine.set_params(&all_on);
    let start = Instant::now();
    let mut sum = 0.0f32;
    for &x in &chord {
        let w = engine.process(x);
        sum += w[0] + w[1];
    }
    let ratio = start.elapsed().as_secs_f32() / SECONDS;
    println!("all features on: {:.2}% of one core at 48 kHz (checksum {sum:.3})", ratio * 100.0);
    Ok(())
}
