use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use seedspace_dsp::{Engine, EngineParams};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

mod editor;

const SIZE_MIN_MS: f32 = 10.0;
const SIZE_MAX_MS: f32 = 2000.0;

#[derive(Default)]
pub struct Seedspace {
    params: Arc<SeedspaceParams>,
    // None until initialize: Engine::new allocates at maximum length and needs the sample rate
    engine: Option<Engine>,
}

#[derive(Enum, PartialEq, Clone, Copy)]
enum Division {
    #[id = "1/2"]
    #[name = "1/2"]
    Half,
    #[id = "1/4"]
    #[name = "1/4"]
    Quarter,
    #[id = "1/4d"]
    #[name = "1/4 dotted"]
    QuarterDotted,
    #[id = "1/4t"]
    #[name = "1/4 triplet"]
    QuarterTriplet,
    #[id = "1/8"]
    #[name = "1/8"]
    Eighth,
    #[id = "1/8d"]
    #[name = "1/8 dotted"]
    EighthDotted,
    #[id = "1/8t"]
    #[name = "1/8 triplet"]
    EighthTriplet,
    #[id = "1/16"]
    #[name = "1/16"]
    Sixteenth,
}

impl Division {
    fn beats(self) -> f32 {
        match self {
            Division::Half => 2.0,
            Division::Quarter => 1.0,
            Division::QuarterDotted => 1.5,
            Division::QuarterTriplet => 2.0 / 3.0,
            Division::Eighth => 0.5,
            Division::EighthDotted => 0.75,
            Division::EighthTriplet => 1.0 / 3.0,
            Division::Sixteenth => 0.25,
        }
    }
}

fn synced_size_ms(division: Division, tempo: Option<f64>) -> f32 {
    let bpm = tempo.unwrap_or(120.0) as f32;
    (division.beats() * 60_000.0 / bpm).clamp(SIZE_MIN_MS, SIZE_MAX_MS)
}

type DiceRanges = BTreeMap<String, (f32, f32, bool)>;

#[derive(Params)]
struct SeedspaceParams {
    #[id = "size"]
    size: FloatParam,
    #[id = "decay"]
    decay: FloatParam,
    #[id = "diffuse"]
    diffuse: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "low_cut"]
    low_cut: FloatParam,
    #[id = "width"]
    width: FloatParam,
    #[id = "freeze"]
    freeze: BoolParam,
    #[id = "sync"]
    sync: BoolParam,
    #[id = "division"]
    division: EnumParam<Division>,

    #[id = "seed"]
    seed: IntParam,
    #[id = "geometry"]
    geometry: FloatParam,
    #[id = "material"]
    material: FloatParam,
    #[id = "motion"]
    motion: FloatParam,
    #[id = "scatter"]
    scatter: FloatParam,

    #[id = "prism"]
    prism: FloatParam,
    #[id = "shimmer"]
    shimmer: FloatParam,
    #[id = "tune"]
    tune: FloatParam,
    #[id = "key"]
    key: IntParam,
    #[id = "bloom"]
    bloom: FloatParam,

    #[id = "mix"]
    mix: FloatParam,
    #[id = "out"]
    out: FloatParam,

    // The GUI stores per-knob dice range and lock here; the audio thread never reads it
    #[persist = "dice-v1"]
    dice: Arc<RwLock<DiceRanges>>,

    #[persist = "editor-state-v1"]
    editor_state: Arc<EguiState>,
}

fn hz_param(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(-2.0),
        },
    )
    .with_smoother(SmoothingStyle::Logarithmic(20.0))
    .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
    .with_string_to_value(formatters::s2v_f32_hz_then_khz())
}

fn percent_param(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(20.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for SeedspaceParams {
    fn default() -> Self {
        let d = EngineParams::default();
        Self {
            size: FloatParam::new(
                "Size",
                d.size_ms,
                FloatRange::Skewed {
                    min: SIZE_MIN_MS,
                    max: SIZE_MAX_MS,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            // Long enough that moving Size glides like tape instead of stepping
            .with_smoother(SmoothingStyle::Linear(150.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            decay: FloatParam::new(
                "Decay",
                d.decay_s,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 30.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            diffuse: percent_param("Diffuse", d.diffuse),
            tone: hz_param("Tone", d.tone_hz, 500.0, 20_000.0),
            low_cut: hz_param("Low Cut", d.low_cut_hz, 20.0, 1000.0),
            width: percent_param("Width", d.width),
            freeze: BoolParam::new("Freeze", d.freeze),
            sync: BoolParam::new("Sync", false),
            division: EnumParam::new("Division", Division::Eighth),

            seed: IntParam::new(
                "Seed",
                d.seed as i32,
                IntRange::Linear { min: 0, max: 9999 },
            ),
            geometry: percent_param("Geometry", d.geometry),
            material: percent_param("Material", d.material),
            motion: percent_param("Motion", d.motion),
            scatter: percent_param("Scatter", d.scatter),

            prism: percent_param("Prism", d.prism),
            shimmer: percent_param("Shimmer", d.shimmer),
            tune: percent_param("Tune", d.tune),
            key: IntParam::new("Key", d.key as i32, IntRange::Linear { min: 0, max: 11 })
                .with_value_to_string(Arc::new(|v| util::NOTES[v as usize].to_string()))
                .with_string_to_value(Arc::new(|s| {
                    util::NOTES
                        .iter()
                        .position(|note| note.eq_ignore_ascii_case(s.trim()))
                        .map(|i| i as i32)
                })),
            bloom: percent_param("Bloom", d.bloom),

            mix: percent_param("Mix", 0.3),
            out: FloatParam::new(
                "Out",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-24.0),
                    max: util::db_to_gain(12.0),
                    factor: FloatRange::gain_skew_factor(-24.0, 12.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),

            dice: Arc::new(RwLock::new(BTreeMap::new())),
            editor_state: EguiState::from_size(860, 520),
        }
    }
}

impl Plugin for Seedspace {
    const NAME: &'static str = "Seedspace";
    const VENDOR: &'static str = "sungha";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.engine = Some(Engine::new(buffer_config.sample_rate));
        true
    }

    fn reset(&mut self) {
        if let Some(engine) = &mut self.engine {
            engine.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(engine) = &mut self.engine else {
            return ProcessStatus::KeepAlive;
        };
        let p = &self.params;
        let tempo = context.transport().tempo;

        for (_, mut block) in buffer.iter_blocks(32) {
            let steps = block.samples() as u32;
            // Advanced under sync too, so the knob's glide doesn't stall until sync is turned off
            let size_ms = p.size.smoothed.next_step(steps);
            engine.set_params(&EngineParams {
                size_ms: if p.sync.value() {
                    synced_size_ms(p.division.value(), tempo)
                } else {
                    size_ms
                },
                decay_s: p.decay.smoothed.next_step(steps),
                diffuse: p.diffuse.smoothed.next_step(steps),
                tone_hz: p.tone.smoothed.next_step(steps),
                low_cut_hz: p.low_cut.smoothed.next_step(steps),
                width: p.width.smoothed.next_step(steps),
                freeze: p.freeze.value(),
                seed: p.seed.value() as u32,
                geometry: p.geometry.smoothed.next_step(steps),
                material: p.material.smoothed.next_step(steps),
                motion: p.motion.smoothed.next_step(steps),
                scatter: p.scatter.smoothed.next_step(steps),
                prism: p.prism.smoothed.next_step(steps),
                shimmer: p.shimmer.smoothed.next_step(steps),
                tune: p.tune.smoothed.next_step(steps),
                key: p.key.value() as u8,
                bloom: p.bloom.smoothed.next_step(steps),
            });

            for mut frame in block.iter_samples() {
                let mix = p.mix.smoothed.next();
                let gain = p.out.smoothed.next();
                let out = |dry: f32, wet: f32| (dry + (wet - dry) * mix) * gain;

                let mut channels = frame.iter_mut();
                match (channels.next(), channels.next()) {
                    (Some(left), Some(right)) => {
                        let dry = [*left, *right];
                        let wet = engine.process(dry);
                        *left = out(dry[0], wet[0]);
                        *right = out(dry[1], wet[1]);
                    }
                    (Some(mono), None) => {
                        let dry = *mono;
                        let wet = engine.process([dry, dry]);
                        *mono = out(dry, (wet[0] + wet[1]) * 0.5);
                    }
                    _ => {}
                }
            }
        }

        // Freeze is an infinite tail
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Seedspace {
    const CLAP_ID: &'static str = "com.sungha.seedspace";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Reverb and delay that draws its space from a seed");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Delay,
        ClapFeature::Reverb,
    ];
}

impl Vst3Plugin for Seedspace {
    const VST3_CLASS_ID: [u8; 16] = *b"SeedspaceSunghaV";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Delay,
        Vst3SubCategory::Reverb,
    ];
}

nih_export_clap!(Seedspace);
nih_export_vst3!(Seedspace);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_falls_back_to_120_bpm() {
        assert_eq!(synced_size_ms(Division::Eighth, None), 250.0);
    }

    #[test]
    fn sync_handles_dotted_and_triplet() {
        assert_eq!(synced_size_ms(Division::QuarterDotted, Some(120.0)), 750.0);
        assert!((synced_size_ms(Division::EighthTriplet, Some(120.0)) - 166.667).abs() < 0.001);
    }

    #[test]
    fn sync_clamps_to_size_range() {
        assert_eq!(synced_size_ms(Division::Half, Some(20.0)), SIZE_MAX_MS);
        assert_eq!(
            synced_size_ms(Division::Sixteenth, Some(2000.0)),
            SIZE_MIN_MS
        );
    }

    #[test]
    fn key_shows_and_parses_note_names() {
        let key = SeedspaceParams::default().key;
        let c_sharp = key.preview_normalized(1);
        assert_eq!(key.normalized_value_to_string(c_sharp, false), "C#");
        assert_eq!(key.string_to_normalized_value(" c# "), Some(c_sharp));
    }
}
