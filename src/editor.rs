use crate::{DiceRanges, Division, SeedspaceParams};
use nih_plug::prelude::*;
use nih_plug_egui::create_egui_editor;
use nih_plug_egui::egui::{
    self, pos2, vec2, Align2, Color32, FontFamily, FontId, Id, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui,
};
use seedspace_dsp::{SpaceDna, LINES};
use std::f32::consts::{PI, TAU};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const BG: Color32 = Color32::from_rgb(0x0e, 0x10, 0x14);
const PANEL: Color32 = Color32::from_rgb(0x17, 0x1a, 0x21);
const EDGE: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x3a);
const TEXT: Color32 = Color32::from_rgb(0xd8, 0xdd, 0xe6);
const TEXT_DIM: Color32 = Color32::from_rgb(0x7a, 0x82, 0x91);
const ACCENT: Color32 = Color32::from_rgb(0x5f, 0xd3, 0xb0);
const RANGE: Color32 = Color32::from_rgb(0xe0, 0xa8, 0x5a);
const RANGE_LOCKED: Color32 = Color32::from_rgb(0x5a, 0x4e, 0x3c);
const PRISM: Color32 = Color32::from_rgb(0xb4, 0x8c, 0xff);
const REVERSE: Color32 = Color32::from_rgb(0xff, 0x6f, 0x91);
const FREEZE: Color32 = Color32::from_rgb(0x7f, 0xc8, 0xff);
// Space Map line colors: lightly damped (long, bright) to heavily damped (short, dark)
const LINE_BRIGHT: Color32 = Color32::from_rgb(0xf2, 0xe6, 0x9a);
const LINE_DARK: Color32 = Color32::from_rgb(0x3a, 0x5a, 0x9a);

const KNOB_START: f32 = 0.75 * PI;
const KNOB_SWEEP: f32 = 1.5 * PI;
const HINT: &str = "Drag ↕ · Shift = fine · Double-click = reset · Amber handles = dice range";

pub(crate) fn create(params: Arc<SeedspaceParams>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        (),
        |ctx, _| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = BG;
            visuals.override_text_color = Some(TEXT);
            ctx.set_visuals(visuals);
            // egui's default proportional font is a Light weight that reads poorly at small sizes
            let mut fonts = egui::FontDefinitions::default();
            for (name, bytes) in [
                (
                    "noto",
                    &include_bytes!("../assets/fonts/NotoSans-Regular.ttf")[..],
                ),
                (
                    "noto-bold",
                    &include_bytes!("../assets/fonts/NotoSans-Bold.ttf")[..],
                ),
            ] {
                fonts
                    .font_data
                    .insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
            }
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "noto".into());
            let fallback = fonts.families[&FontFamily::Proportional].clone();
            fonts.families.insert(
                FontFamily::Name("bold".into()),
                [vec!["noto-bold".into()], fallback].concat(),
            );
            ctx.set_fonts(fonts);
        },
        move |ctx, setter, _| {
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(BG).inner_margin(0))
                .show(ctx, |ui| draw(ui, &params, setter));
        },
    )
}

fn draw(ui: &mut Ui, p: &SeedspaceParams, setter: &ParamSetter) {
    let full = ui.max_rect();
    let o = full.min;
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(o + vec2(x, y), vec2(w, h));

    // Header
    let painter = ui.painter().clone();
    painter.line_segment(
        [o + vec2(0.0, 56.0), o + vec2(full.width(), 56.0)],
        Stroke::new(1.0_f32, EDGE),
    );
    painter.text(
        o + vec2(20.0, 28.0),
        Align2::LEFT_CENTER,
        "SEEDSPACE",
        bold(20.0),
        TEXT,
    );
    if toggle(
        ui,
        at(180.0, 14.0, 76.0, 28.0),
        "dice",
        "DICE",
        true,
        ACCENT,
    ) {
        dice(p, setter);
    }
    let seed = p.seed.unmodulated_plain_value();
    for (x, step) in [(266.0, -1), (392.0, 1)] {
        let rect = at(x, 16.0, 20.0, 24.0);
        if toggle(ui, rect, &format!("seed{step}"), "", false, ACCENT) {
            setter.begin_set_parameter(&p.seed);
            setter.set_parameter(&p.seed, step_seed(seed, step));
            setter.end_set_parameter(&p.seed);
        }
        let c = rect.center();
        let d = step as f32 * 4.0;
        painter.add(Shape::convex_polygon(
            vec![c + vec2(d, 0.0), c + vec2(-d, -5.0), c + vec2(-d, 5.0)],
            TEXT_DIM,
            Stroke::NONE,
        ));
    }
    painter.text(
        o + vec2(334.0, 28.0),
        Align2::CENTER_CENTER,
        format!("SEED {seed:04}"),
        FontId::monospace(16.0),
        TEXT,
    );
    lock_toggle(ui, o + vec2(430.0, 28.0), "seed", p);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(at(470.0, 16.0, 190.0, 24.0)));
    egui::ComboBox::from_id_salt("preset")
        .width(180.0)
        .selected_text("PRESETS")
        .show_ui(&mut child, |ui| {
            for (i, (name, _)) in PRESETS.iter().enumerate() {
                if ui.selectable_label(false, *name).clicked() {
                    load_preset(p, setter, i);
                }
            }
        });
    if toggle(
        ui,
        at(full.width() - 116.0, 14.0, 96.0, 28.0),
        "freeze",
        "FREEZE",
        p.freeze.value(),
        FREEZE,
    ) {
        flip(setter, &p.freeze);
    }

    // Each row: a 16 px title strip, then a 124 px card
    let card = |x: f32, row: usize, w: f32, title: &str| {
        let rect = at(x, 80.0 + row as f32 * 148.0, w, 124.0);
        heading(&painter, rect.min + vec2(4.0, -9.0), title);
        painter.rect_filled(rect, 6.0_f32, PANEL);
        painter.rect_stroke(
            rect,
            6.0_f32,
            Stroke::new(1.0_f32, EDGE),
            StrokeKind::Inside,
        );
        rect
    };
    let cell = |card: Rect, n: usize, i: usize| {
        let w = card.width() / n as f32;
        Rect::from_min_size(card.min + vec2(w * i as f32, 0.0), vec2(w, card.height()))
    };

    let map = at(16.0, 80.0, 264.0, 420.0);
    heading(&painter, map.min + vec2(4.0, -9.0), "SPACE MAP");
    space_map(ui, map, p);

    let space = card(296.0, 0, 548.0, "SPACE");
    knob(ui, cell(space, 6, 0), "size", &p.size, setter, p);
    knob(ui, cell(space, 6, 1), "decay", &p.decay, setter, p);
    knob(ui, cell(space, 6, 2), "diffuse", &p.diffuse, setter, p);
    knob(ui, cell(space, 6, 3), "tone", &p.tone, setter, p);
    knob(ui, cell(space, 6, 4), "low_cut", &p.low_cut, setter, p);
    knob(ui, cell(space, 6, 5), "width", &p.width, setter, p);
    let random = card(296.0, 1, 360.0, "RANDOM");
    knob(ui, cell(random, 4, 0), "geometry", &p.geometry, setter, p);
    knob(ui, cell(random, 4, 1), "material", &p.material, setter, p);
    knob(ui, cell(random, 4, 2), "motion", &p.motion, setter, p);
    knob(ui, cell(random, 4, 3), "scatter", &p.scatter, setter, p);
    let output = card(664.0, 1, 180.0, "OUTPUT");
    knob(ui, cell(output, 2, 0), "mix", &p.mix, setter, p);
    knob(ui, cell(output, 2, 1), "out", &p.out, setter, p);
    let sound = card(296.0, 2, 450.0, "SOUND");
    knob(ui, cell(sound, 5, 0), "prism", &p.prism, setter, p);
    knob(ui, cell(sound, 5, 1), "shimmer", &p.shimmer, setter, p);
    knob(ui, cell(sound, 5, 2), "tune", &p.tune, setter, p);
    knob(ui, cell(sound, 5, 3), "key", &p.key, setter, p);
    knob(ui, cell(sound, 5, 4), "bloom", &p.bloom, setter, p);
    let time = card(754.0, 2, 90.0, "TIME");

    let sync_rect = Rect::from_min_size(time.min + vec2(10.0, 30.0), vec2(70.0, 28.0));
    if toggle(ui, sync_rect, "sync", "SYNC", p.sync.value(), ACCENT) {
        flip(setter, &p.sync);
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
        sync_rect.left_bottom() + vec2(0.0, 12.0),
        vec2(70.0, 28.0),
    )));
    let current = p.division.value().to_index();
    egui::ComboBox::from_id_salt("division")
        .width(70.0)
        .selected_text(Division::variants()[current])
        .show_ui(&mut child, |ui| {
            for (i, name) in Division::variants().iter().enumerate() {
                if ui.selectable_label(i == current, *name).clicked() {
                    setter.begin_set_parameter(&p.division);
                    setter.set_parameter(&p.division, Division::from_index(i));
                    setter.end_set_parameter(&p.division);
                }
            }
        });
}

fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

/// Letter-spaced section title, left-center anchored.
fn heading(painter: &egui::Painter, pos: Pos2, text: &str) {
    let format = egui::TextFormat {
        font_id: bold(11.0),
        color: TEXT_DIM,
        extra_letter_spacing: 1.2,
        ..Default::default()
    };
    let galley = painter.layout_job(egui::text::LayoutJob::single_section(
        text.to_string(),
        format,
    ));
    painter.galley(pos - vec2(0.0, galley.size().y * 0.5), galley, TEXT_DIM);
}

fn step_seed(seed: i32, step: i32) -> i32 {
    (seed + step).rem_euclid(10_000)
}

fn space_map(ui: &mut Ui, rect: Rect, p: &SeedspaceParams) {
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0_f32, PANEL);
    painter.rect_stroke(
        rect,
        6.0_f32,
        Stroke::new(1.0_f32, EDGE),
        StrokeKind::Inside,
    );

    let dna = SpaceDna::from_seed(p.seed.unmodulated_plain_value() as u32);
    let ms = dna.line_ms(
        p.size.unmodulated_plain_value(),
        p.geometry.unmodulated_plain_value(),
        p.diffuse.unmodulated_plain_value(),
    );
    // Normalized to the longest line: against the 2 s maximum a 120 ms space would be a dot
    let longest = ms.iter().copied().fold(f32::EPSILON, f32::max);
    let center = rect.center() + vec2(0.0, -10.0);
    let radius = 110.0;
    let material = p.material.unmodulated_plain_value();
    let prism = p.prism.unmodulated_plain_value();

    painter.circle_stroke(center, radius, Stroke::new(1.0_f32, EDGE));
    let mut tips: Vec<(f32, Pos2)> = (0..LINES)
        .map(|i| {
            // Pan -1..1 sweeps from lower left over the top to lower right
            let angle = -0.5 * PI + dna.pan[i] * 0.85 * PI;
            (
                angle,
                center + vec2(angle.cos(), angle.sin()) * radius * ms[i] / longest,
            )
        })
        .collect();
    for (i, &(_, tip)) in tips.iter().enumerate() {
        let darkness = ((material * dna.damp[i] + 1.0) * 0.5).clamp(0.0, 1.0);
        let color = LINE_BRIGHT.lerp_to_gamma(LINE_DARK, darkness);
        painter.line_segment([center, tip], Stroke::new(3.0_f32, color));
        painter.circle_filled(tip, 4.0, color);
        if dna.prism_entry[i] < prism {
            let mark = if dna.prism_ratio[i] < 0.0 {
                REVERSE
            } else {
                PRISM
            };
            painter.circle_stroke(tip, 8.0, Stroke::new(2.0_f32, mark));
        }
    }
    tips.sort_by(|a, b| a.0.total_cmp(&b.0));
    painter.add(Shape::closed_line(
        tips.iter().map(|t| t.1).collect(),
        Stroke::new(1.0_f32, TEXT_DIM.gamma_multiply(0.6)),
    ));
    painter.circle_filled(center, 3.0, TEXT_DIM);

    let legend_y = rect.max.y - 18.0;
    let font = FontId::proportional(10.0);
    painter.circle_stroke(
        pos2(rect.min.x + 18.0, legend_y),
        5.0,
        Stroke::new(2.0_f32, PRISM),
    );
    painter.text(
        pos2(rect.min.x + 28.0, legend_y),
        Align2::LEFT_CENTER,
        "PRISM",
        font.clone(),
        TEXT_DIM,
    );
    painter.circle_stroke(
        pos2(rect.min.x + 86.0, legend_y),
        5.0,
        Stroke::new(2.0_f32, REVERSE),
    );
    painter.text(
        pos2(rect.min.x + 96.0, legend_y),
        Align2::LEFT_CENTER,
        "REVERSE",
        font.clone(),
        TEXT_DIM,
    );
    painter.text(
        pos2(rect.max.x - 12.0, legend_y),
        Align2::RIGHT_CENTER,
        format!("{:.0} ms", longest),
        font,
        TEXT_DIM,
    );
}

fn knob<P: Param>(
    ui: &mut Ui,
    cell: Rect,
    id: &str,
    param: &P,
    setter: &ParamSetter,
    p: &SeedspaceParams,
) {
    let center = cell.min + vec2(cell.width() * 0.5, 60.0);
    let response = ui.interact(
        Rect::from_center_size(center, vec2(48.0, 48.0)),
        Id::new(id),
        Sense::click_and_drag(),
    );
    let fine = if ui.input(|i| i.modifiers.shift) {
        0.1
    } else {
        1.0
    };

    // Tracked unsnapped: reading the param back would undo small moves on stepped params
    let drag_id = response.id.with("drag");
    if response.drag_started() {
        setter.begin_set_parameter(param);
        let start = param.unmodulated_normalized_value();
        ui.memory_mut(|m| m.data.insert_temp(drag_id, start));
    }
    if response.dragged() {
        let current: f32 = ui
            .memory(|m| m.data.get_temp(drag_id))
            .unwrap_or_else(|| param.unmodulated_normalized_value());
        let normalized = (current - response.drag_delta().y * 0.005 * fine).clamp(0.0, 1.0);
        ui.memory_mut(|m| m.data.insert_temp(drag_id, normalized));
        setter.set_parameter_normalized(param, normalized);
    }
    if response.drag_stopped() {
        setter.end_set_parameter(param);
    }
    // baseview hosts don't always report double clicks, so also time consecutive clicks
    let mut double_clicked = response.double_clicked();
    if response.clicked() {
        let now = ui.input(|i| i.time);
        let last: Option<f64> = ui.memory_mut(|m| {
            let last = m.data.get_temp(response.id);
            m.data.insert_temp(response.id, now);
            last
        });
        double_clicked |= last.is_some_and(|t| now - t < 0.4);
    }
    let scroll = ui.input(|i| i.raw_scroll_delta.y);
    if response.hovered() && scroll != 0.0 {
        // Stepped params move one step per wheel event; smaller moves would snap back
        let delta = param
            .step_count()
            .map_or(scroll * 0.002 * fine, |n| scroll.signum() / n as f32);
        setter.begin_set_parameter(param);
        setter.set_parameter_normalized(
            param,
            (param.unmodulated_normalized_value() + delta).clamp(0.0, 1.0),
        );
        setter.end_set_parameter(param);
    }
    if double_clicked {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, param.default_plain_value());
        setter.end_set_parameter(param);
    }

    // Dice range handles, registered after the body so they win the hit test
    let decay_10s = p.decay.preview_normalized(10.0);
    let (mut lo, mut hi, locked) = entry(&p.dice.read().unwrap(), id, decay_10s);
    let on_arc = |t: f32, r: f32| {
        let a = KNOB_START + KNOB_SWEEP * t;
        center + vec2(a.cos(), a.sin()) * r
    };
    let mut changed = false;
    for (end, value) in [("lo", &mut lo), ("hi", &mut hi)] {
        let handle = ui.interact(
            Rect::from_center_size(on_arc(*value, 32.0), vec2(20.0, 20.0)),
            Id::new(id).with(end),
            Sense::drag(),
        );
        if let Some(pos) = handle.interact_pointer_pos().filter(|_| handle.dragged()) {
            *value = angle_to_norm(pos - center);
            changed = true;
        }
    }
    if changed {
        // Handles that cross swap roles, so overlapping ones can always pull apart
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        p.dice
            .write()
            .unwrap()
            .insert(id.to_string(), (lo, hi, locked));
    }

    let painter = ui.painter();
    let arc = |from: f32, to: f32, r: f32| -> Vec<Pos2> {
        let n = ((to - from) * 40.0).ceil().max(1.0) as usize;
        (0..=n)
            .map(|k| on_arc(from + (to - from) * k as f32 / n as f32, r))
            .collect()
    };
    painter.text(
        cell.min + vec2(cell.width() * 0.5, 16.0),
        Align2::CENTER_CENTER,
        param.name().to_uppercase(),
        bold(11.0),
        TEXT_DIM,
    );
    let range_color = if locked { RANGE_LOCKED } else { RANGE };
    painter.add(Shape::line(
        arc(lo, hi, 32.0),
        Stroke::new(2.0_f32, range_color),
    ));
    for t in [lo, hi] {
        painter.circle_filled(on_arc(t, 32.0), 4.0, range_color);
    }
    let norm = param.unmodulated_normalized_value();
    painter.add(Shape::line(arc(0.0, 1.0, 23.0), Stroke::new(3.0_f32, EDGE)));
    painter.add(Shape::line(
        arc(0.0, norm, 23.0),
        Stroke::new(3.0_f32, ACCENT),
    ));
    painter.circle_filled(center, 17.0, BG);
    painter.circle_stroke(
        center,
        17.0,
        Stroke::new(1.0_f32, if response.hovered() { TEXT_DIM } else { EDGE }),
    );
    painter.line_segment([center, on_arc(norm, 14.0)], Stroke::new(2.0_f32, TEXT));
    let value = param.normalized_value_to_string(norm, true);
    painter.text(
        cell.min + vec2(cell.width() * 0.5, 106.0),
        Align2::CENTER_CENTER,
        &value,
        FontId::proportional(13.0),
        TEXT,
    );
    response.on_hover_text(format!("{} {value}\n{HINT}", param.name()));

    // In the arc's bottom gap: the only spot no dice handle can ever reach
    lock_toggle(ui, center + vec2(0.0, 30.0), id, p);
}

/// Painted padlock centered at `center`; closed and amber when Dice skips `id`.
fn lock_toggle(ui: &mut Ui, center: Pos2, id: &str, p: &SeedspaceParams) {
    let decay_10s = p.decay.preview_normalized(10.0);
    let (lo, hi, locked) = entry(&p.dice.read().unwrap(), id, decay_10s);
    let response = ui
        .interact(
            Rect::from_center_size(center, vec2(20.0, 20.0)),
            Id::new(id).with("lock"),
            Sense::click(),
        )
        .on_hover_text(if locked {
            "Locked: Dice skips this"
        } else {
            "Lock to keep this when rolling Dice"
        });
    if response.clicked() {
        p.dice
            .write()
            .unwrap()
            .insert(id.to_string(), (lo, hi, !locked));
    }
    let color = if locked {
        RANGE
    } else if response.hovered() {
        TEXT
    } else {
        TEXT_DIM
    };
    let painter = ui.painter();
    let body = Rect::from_center_size(center + vec2(0.0, 2.5), vec2(10.0, 7.0));
    painter.rect_filled(body, 1.5_f32, color);
    // Open shackle floats above the body
    let base = body.min.y - if locked { 0.0 } else { 2.5 };
    let c = pos2(center.x, base - 2.5);
    let shackle: Vec<Pos2> = std::iter::once(pos2(c.x - 3.0, base))
        .chain((0..=8).map(|k| {
            let a = PI + k as f32 * PI / 8.0;
            c + vec2(a.cos(), a.sin()) * 3.0
        }))
        .chain(std::iter::once(pos2(c.x + 3.0, base)))
        .collect();
    painter.add(Shape::line(shackle, Stroke::new(1.5_f32, color)));
}

/// Painted button; returns true when clicked.
fn toggle(ui: &mut Ui, rect: Rect, id: &str, label: &str, on: bool, color: Color32) -> bool {
    let response = ui.interact(rect, Id::new(id), Sense::click());
    let painter = ui.painter();
    let edge = if on {
        color
    } else if response.hovered() {
        TEXT_DIM
    } else {
        EDGE
    };
    painter.rect_filled(rect, 4.0_f32, if on { color } else { PANEL });
    painter.rect_stroke(
        rect,
        4.0_f32,
        Stroke::new(1.0_f32, edge),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        bold((rect.height() * 0.45).max(9.0)),
        if on { BG } else { TEXT },
    );
    response.clicked()
}

fn flip(setter: &ParamSetter, param: &BoolParam) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, !param.value());
    setter.end_set_parameter(param);
}

/// Inverse of the knob's arc; the gap at the bottom snaps to the nearer end.
fn angle_to_norm(v: egui::Vec2) -> f32 {
    let t = (v.y.atan2(v.x) - KNOB_START).rem_euclid(TAU) / KNOB_SWEEP;
    if t <= 1.0 {
        t
    } else if t < 0.5 + TAU / KNOB_SWEEP * 0.5 {
        1.0
    } else {
        0.0
    }
}

fn default_entry(id: &str, decay_10s: f32) -> (f32, f32, bool) {
    match id {
        // Volume jumps and broken tempo are never what Dice is for
        "mix" | "out" | "freeze" | "sync" | "division" | "key" => (0.0, 1.0, true),
        "decay" => (0.0, decay_10s, false),
        // Capped so a roll doesn't pile every effect on at full strength
        "prism" | "shimmer" | "scatter" | "tune" | "bloom" => (0.0, 0.5, false),
        _ => (0.0, 1.0, false),
    }
}

fn entry(dice: &DiceRanges, id: &str, decay_10s: f32) -> (f32, f32, bool) {
    dice.get(id)
        .copied()
        .unwrap_or_else(|| default_entry(id, decay_10s))
}

fn xorshift(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

/// New normalized values for every unlocked id, uniform within its range. `state` must be nonzero.
fn roll(ids: &[String], dice: &DiceRanges, decay_10s: f32, mut state: u32) -> Vec<(String, f32)> {
    ids.iter()
        .filter_map(|id| {
            let (lo, hi, locked) = entry(dice, id, decay_10s);
            (!locked).then(|| (id.clone(), lo + (hi - lo) * xorshift(&mut state)))
        })
        .collect()
}

fn dice(p: &SeedspaceParams, setter: &ParamSetter) {
    let map = p.param_map();
    let ids: Vec<String> = map.iter().map(|(id, _, _)| id.clone()).collect();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| d.subsec_nanos());
    // Spread the time bits and avoid the all-zero state xorshift never leaves
    let state = nanos.wrapping_mul(0x9e37_79b9) | 1;
    let rolled = roll(
        &ids,
        &p.dice.read().unwrap(),
        p.decay.preview_normalized(10.0),
        state,
    );
    for (id, value) in rolled {
        let Some((_, ptr, _)) = map.iter().find(|(i, _, _)| *i == id) else {
            continue;
        };
        // SAFETY: ptr comes from p.param_map() and the params Arc outlives this call
        unsafe {
            setter.raw_context.raw_begin_set_parameter(*ptr);
            setter.raw_context.raw_set_parameter_normalized(*ptr, value);
            setter.raw_context.raw_end_set_parameter(*ptr);
        }
    }
}

/// Plain values by param id; anything unlisted loads its default. Seeds are part of the sound.
const PRESETS: &[(&str, &[(&str, f32)])] = &[
    ("Init", &[]),
    ("Small Room", &[("size", 40.0), ("decay", 0.8), ("diffuse", 1.0), ("tone", 6000.0), ("geometry", 0.8), ("material", 0.2), ("motion", 0.1), ("mix", 0.25)]),
    ("Big Hall", &[("size", 300.0), ("decay", 6.0), ("diffuse", 0.95), ("tone", 7000.0), ("low_cut", 120.0), ("geometry", 0.7), ("material", 0.4), ("motion", 0.25), ("mix", 0.35)]),
    ("Polyrhythm Echo", &[("size", 500.0), ("decay", 4.0), ("diffuse", 0.0), ("tone", 5000.0), ("geometry", 0.0), ("material", 0.2), ("motion", 0.05), ("sync", 1.0), ("division", 1.0), ("mix", 0.35)]),
    ("Tape Smear", &[("size", 250.0), ("decay", 3.0), ("diffuse", 0.4), ("tone", 3500.0), ("motion", 0.6), ("scatter", 0.2), ("mix", 0.35)]),
    ("Shimmer Cathedral", &[("size", 400.0), ("decay", 12.0), ("diffuse", 1.0), ("tone", 9000.0), ("low_cut", 200.0), ("shimmer", 0.6), ("motion", 0.2), ("mix", 0.4)]),
    ("Prism Cloud", &[("seed", 2961.0), ("size", 250.0), ("decay", 8.0), ("diffuse", 0.9), ("prism", 0.7), ("motion", 0.3), ("mix", 0.4)]),
    ("Reverse Ghost", &[("seed", 9747.0), ("size", 300.0), ("decay", 5.0), ("diffuse", 0.8), ("prism", 0.6), ("mix", 0.4)]),
    ("Broken Mirror", &[("size", 600.0), ("decay", 4.0), ("diffuse", 0.5), ("geometry", 1.0), ("scatter", 0.8), ("mix", 0.35)]),
    ("Chord Room", &[("size", 150.0), ("decay", 4.0), ("tune", 0.7), ("key", 9.0), ("mix", 0.35)]),
    ("Breathing Room", &[("size", 300.0), ("decay", 5.0), ("bloom", 0.8), ("motion", 0.2), ("mix", 0.4)]),
];

/// Sets every param except Out, so a preset never changes the user's level.
fn load_preset(p: &SeedspaceParams, setter: &ParamSetter, index: usize) {
    let values = PRESETS[index].1;
    for (id, ptr, _) in p.param_map() {
        if id == "out" {
            continue;
        }
        // SAFETY: ptr comes from p.param_map() and the params Arc outlives this call
        unsafe {
            let normalized = match values.iter().find(|(i, _)| *i == id) {
                Some(&(_, plain)) => ptr.preview_normalized(plain),
                None => ptr.default_normalized_value(),
            };
            setter.raw_context.raw_begin_set_parameter(ptr);
            setter.raw_context.raw_set_parameter_normalized(ptr, normalized);
            setter.raw_context.raw_end_set_parameter(ptr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn presets_name_real_params_within_range() {
        let params = SeedspaceParams::default();
        let map = params.param_map();
        for (name, values) in PRESETS {
            for &(id, plain) in *values {
                let (_, ptr, _) = map.iter().find(|(i, _, _)| i == id).unwrap_or_else(|| panic!("{name}: no param {id}"));
                // SAFETY: params outlives the pointer
                let back = unsafe { ptr.preview_plain(ptr.preview_normalized(plain)) };
                assert!((back - plain).abs() <= plain.abs() * 1e-3 + 1e-3, "{name}: {id} {plain} clamps to {back}");
            }
        }
    }

    fn ids() -> Vec<String> {
        SeedspaceParams::default()
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect()
    }

    #[test]
    fn empty_map_rolls_defaults() {
        let decay_10s = SeedspaceParams::default().decay.preview_normalized(10.0);
        let ids = ids();
        assert_eq!(ids.len(), 21);
        for state in [1, 0xdead_beef, 12345] {
            let rolled = roll(&ids, &BTreeMap::new(), decay_10s, state);
            assert_eq!(rolled.len(), 15);
            for (id, v) in &rolled {
                assert!(!["mix", "out", "freeze", "sync", "division", "key"].contains(&id.as_str()));
                let hi = match id.as_str() {
                    "decay" => decay_10s,
                    "prism" | "shimmer" | "scatter" | "tune" | "bloom" => 0.5,
                    _ => 1.0,
                };
                assert!((0.0..=hi).contains(v), "{id} = {v}");
            }
        }
    }

    #[test]
    fn roll_respects_stored_ranges_and_locks() {
        let dice = BTreeMap::from([
            ("size".to_string(), (0.2, 0.4, false)),
            ("seed".to_string(), (0.0, 1.0, true)),
            ("mix".to_string(), (0.6, 0.7, false)),
        ]);
        let mut state: u32 = 7;
        for _ in 0..100 {
            state = state.wrapping_mul(747_796_405).wrapping_add(1) | 1;
            let rolled: BTreeMap<_, _> = roll(&ids(), &dice, 0.75, state).into_iter().collect();
            assert!(!rolled.contains_key("seed"));
            assert!((0.2..=0.4).contains(&rolled["size"]));
            assert!((0.6..=0.7).contains(&rolled["mix"]));
        }
    }

    #[test]
    fn decay_default_is_ten_seconds() {
        let decay = SeedspaceParams::default().decay;
        let hi = default_entry("decay", decay.preview_normalized(10.0)).1;
        assert!((decay.preview_plain(hi) - 10.0).abs() < 1e-3);
    }

    #[test]
    fn seed_steps_wrap() {
        assert_eq!(step_seed(0, -1), 9999);
        assert_eq!(step_seed(9999, 1), 0);
        assert_eq!(step_seed(1234, 1), 1235);
    }

    #[test]
    fn angle_maps_back_to_knob_position() {
        for t in [0.0, 0.25, 0.5, 1.0] {
            let a = KNOB_START + KNOB_SWEEP * t;
            assert!((angle_to_norm(vec2(a.cos(), a.sin())) - t).abs() < 1e-4);
        }
        // Straight down is in the gap: just right of it snaps to 1, just left to 0
        assert_eq!(angle_to_norm(vec2(0.1, 1.0)), 1.0);
        assert_eq!(angle_to_norm(vec2(-0.1, 1.0)), 0.0);
    }
}
