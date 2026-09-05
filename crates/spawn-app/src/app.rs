// SPDX-License-Identifier: GPL-3.0-or-later

use crate::device::Candidate;
use crate::theme::{HUE, key_face, travel_color};
use crate::worker::{Cmd, Event, Worker};
use gpui::{
    AnyElement, Context, ElementId, MouseButton, Point, SharedString, Window, div, prelude::*, px,
};
use spawn_protocol::{
    DeviceInfo, GameMode, KEY_SLOTS, KeyAction, LedEffect, Page, ResetScope, Rgb as DevRgb, RtKey,
    layout::{BOARD_UNITS, KeyCap, SPAWN_ROWS, key_by_slot},
};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use vampir::{
    ButtonVariant, ChipSelection, ComboId, ControlHost, ControlState, MenuItem, Palette,
    SMALL_TEXT_SIZE, SWITCH_SLIDE, SliderTrack, TEXT_SIZE, TITLE_TEXT_SIZE, Tooltip, arriving,
    button, caption, chip, chip_group, collapsible,
    color::{channels, lerp},
    combo,
    containers::Tab as VTab,
    context_menu,
    lighting::{lit, lit_mix, raised, rim, shade},
    menu_button, scheme_picker, segmented, separator, slider,
    swatch::{MAX_CHROMA, Oklch, color_pad, hue_slider},
    switch, tab_bar, ui_font,
};

// ------------------------------------------------------------------ constants

/// The top-level split: the board and everything about the keys, or the
/// device's own settings.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Keyboard,
    Settings,
}

impl Section {
    #[cfg(test)]
    const ALL: [Section; 2] = [Section::Keyboard, Section::Settings];
}

/// What the panel under the board is showing. The board itself stays put
/// across these and only re-colours.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Actuation,
    Keys,
    Lighting,
}

impl Pane {
    const ALL: [Pane; 3] = [Pane::Actuation, Pane::Keys, Pane::Lighting];
    fn title(self) -> &'static str {
        match self {
            Pane::Actuation => "Actuation",
            Pane::Keys => "Keys",
            Pane::Lighting => "Lighting",
        }
    }
}

/// Polling rate is a firmware code, not an index.
const REPORT_RATES: [(u8, &str); 3] = [(3, "1000 Hz"), (5, "4000 Hz"), (6, "8000 Hz")];

/// Magnetic switch models this board can be calibrated for, with the
/// rapid-trigger ceiling each one allows.
///
/// The names are the vendor's own English strings. Its configuration lists
/// these as `axis_6`, `axis_7`, `axis_10`, `axis_20`, `jade` and `king`,
/// which are translation keys rather than names — a label of "Axis 6" tells
/// nobody which switch is under their keys.
///
/// The ceiling comes from the selected switch, not from the board: the two
/// magnetic ones stop a tenth of a millimetre short of the rest.
const AXIS_PROFILES: [(u8, &str, f32); 6] = [
    (1, "Graywood / Yingyi / Yongchun", 3.4),
    (4, "Black Emperor", 3.4),
    (5, "Spirit Cloud", 3.4),
    (6, "Neptune", 3.4),
    (2, "Magnetic Jade Pro", 3.3),
    (3, "Magneto", 3.3),
];

/// The rapid-trigger ceiling for a switch profile.
fn max_rt_for(axis: u8) -> f32 {
    AXIS_PROFILES
        .iter()
        .find(|(value, _, _)| *value == axis)
        .map_or(MAX_RT, |(_, _, max)| *max)
}

struct LedMode {
    value: u8,
    name: &'static str,
    speed: bool,
    color: bool,
}

/// Lighting effects this board implements.
///
/// The vendor application starts from a default list and only adds the extra
/// effects (23, 24, 25, 254) when the board's own configuration names them
/// under `customEffect`. The SPAWN declares no lighting configuration at all,
/// so those extras are not valid here — selecting one leaves the keyboard
/// dark.
const LED_MODES: &[LedMode] = &[
    LedMode {
        value: 1,
        name: "Static",
        speed: false,
        color: true,
    },
    LedMode {
        value: 2,
        name: "Point on",
        speed: true,
        color: true,
    },
    LedMode {
        value: 3,
        name: "Point off",
        speed: true,
        color: true,
    },
    LedMode {
        value: 4,
        name: "Starry sky",
        speed: true,
        color: true,
    },
    LedMode {
        value: 5,
        name: "Snowfall",
        speed: true,
        color: true,
    },
    LedMode {
        value: 6,
        name: "Floral",
        speed: true,
        color: false,
    },
    LedMode {
        value: 7,
        name: "Breathing",
        speed: true,
        color: true,
    },
    LedMode {
        value: 8,
        name: "Spectrum",
        speed: true,
        color: false,
    },
    LedMode {
        value: 9,
        name: "Fountain",
        speed: true,
        color: true,
    },
    LedMode {
        value: 10,
        name: "Interchange",
        speed: true,
        color: true,
    },
    LedMode {
        value: 11,
        name: "Waves",
        speed: true,
        color: true,
    },
    LedMode {
        value: 12,
        name: "Peaks",
        speed: true,
        color: true,
    },
    LedMode {
        value: 13,
        name: "Fire",
        speed: true,
        color: true,
    },
    LedMode {
        value: 14,
        name: "Two birds",
        speed: true,
        color: true,
    },
    LedMode {
        value: 15,
        name: "Ripples",
        speed: true,
        color: true,
    },
    LedMode {
        value: 16,
        name: "Endless flow",
        speed: true,
        color: true,
    },
    LedMode {
        value: 17,
        name: "Mountains",
        speed: true,
        color: true,
    },
    LedMode {
        value: 18,
        name: "Rain",
        speed: true,
        color: true,
    },
    LedMode {
        value: 19,
        name: "Back and forth",
        speed: true,
        color: true,
    },
    LedMode {
        value: CUSTOM_LIGHTING,
        name: "Per-key",
        speed: false,
        color: true,
    },
];

/// Per-key lighting is its own mode value, not an index into the list above.
const CUSTOM_LIGHTING: u8 = 128;
/// The firmware's own "backlight off" state.
const LIGHTS_OFF: u8 = 0;

/// `color_mode` values. 0 uses the RGB in the record; 1 lets the firmware
/// choose, which shows as a rainbow and ignores the colour entirely.
const COLOR_CUSTOM: u8 = 0;
const COLOR_AUTO: u8 = 1;

/// Direction values, as the vendor's arrow buttons send them. It shows two
/// arrows, not four, and which pair depends on the effect: one that travels
/// vertically offers up and down, one that travels horizontally offers left
/// and right. Naming them by index would have got all four wrong.
const DIR_RIGHT: u8 = 0;
const DIR_LEFT: u8 = 1;
const DIR_UP: u8 = 2;
const DIR_DOWN: u8 = 3;

/// The pair of directions an effect offers, or none if it has no direction.
fn directions_for(mode: u8) -> Option<[(u8, &'static str); 2]> {
    match mode {
        // The only vertical effect on this board.
        10 => Some([(DIR_UP, "Up"), (DIR_DOWN, "Down")]),
        11 | 12 | 16 | 18 => Some([(DIR_LEFT, "Left"), (DIR_RIGHT, "Right")]),
        _ => None,
    }
}

/// Brightness and speed both run 1..=5. The vendor reads them as
/// `minSpeed || 1` / `maxSpeed || 5` from the board's lighting configuration,
/// and the SPAWN declares none, so the defaults stand. Values above the top
/// of the range are not a wider range: the firmware misreads them.
/// One keyboard unit on screen, and the gap between caps.
pub const UNIT: f32 = 46.0;
pub const GAP: f32 = 5.0;
/// Padding inside the board's own surface, and around the scrolling body.
const BOARD_PAD: f32 = 16.0;
const BODY_PAD: f32 = 20.0;

/// How wide the window has to be for the board to fit without clipping.
///
/// Every row is [`BOARD_UNITS`] wide however many caps it is divided into:
/// the gaps taken out of the caps are exactly the gaps put back between
/// them, so the row is the same width whether it holds ten keys or fifteen.
pub fn board_width() -> f32 {
    let row = UNIT * BOARD_UNITS + GAP * (BOARD_UNITS - 1.0);
    row + 2.0 * BOARD_PAD + 2.0 * BODY_PAD
}

/// Dead zones run 0..=0.5 mm, at the same 0.01 step as everything else.
/// The vendor's own slider is bounded there; 0.3 is what it fills in when
/// the feature is switched on.
const MAX_DEAD_ZONE: f32 = 0.5;

const MIN_LEVEL: f32 = 1.0;
const MAX_LEVEL: f32 = 5.0;

// Firmware limits, taken from the board's own configuration.
const MIN_TRIGGER: f32 = 0.10;
const MAX_TRIGGER: f32 = 3.40;
const MIN_RT: f32 = 0.01;
const MAX_RT: f32 = 3.40;
/// The firmware stores travel in hundredths of a millimetre.
const STEP: f32 = 0.01;

fn round_step(v: f32) -> f32 {
    (v / STEP).round() * STEP
}

/// The value a slider at `t` of its track stands for.
/// A lighting level from a track position.
///
/// A drag is read straight off the track. A key press is not: Vampir 0.1.2
/// divides a stepped track by `stops` for the keyboard while the tick marks
/// and the mouse divide it by `stops - 1`, so an arrow lands between the
/// positions and, on a five-value scale, rounds back to where it started.
/// Taking the direction rather than the ratio steps reliably, and keeps
/// working whichever way that is settled upstream.
fn stepped_level(dragging: bool, at_x: f32, current: u8) -> u8 {
    let (min, max) = (MIN_LEVEL as u8, MAX_LEVEL as u8);
    if dragging {
        return (value_at(MIN_LEVEL, MAX_LEVEL, at_x).round() as u8).clamp(min, max);
    }
    let here = ratio(f32::from(current), MIN_LEVEL, MAX_LEVEL);
    if at_x <= 0.0 {
        min
    } else if at_x >= 1.0 {
        max
    } else if at_x < here {
        current.saturating_sub(1).max(min)
    } else if at_x > here {
        (current + 1).min(max)
    } else {
        current
    }
}

fn value_at(min: f32, max: f32, t: f32) -> f32 {
    min + (max - min) * t.clamp(0.0, 1.0)
}

fn ratio(value: f32, min: f32, max: f32) -> f32 {
    ((value - min) / (max - min).max(f32::EPSILON)).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------- state

pub struct SpawnApp {
    controls: ControlState,
    worker: Worker,
    devices: Vec<Candidate>,
    connected: Option<String>,
    info: Option<DeviceInfo>,
    game_mode: GameMode,
    rt: Vec<RtKey>,
    led: LedEffect,
    custom_led: Vec<DevRgb>,
    keys: Vec<KeyAction>,
    selection: HashSet<u8>,
    travel: HashMap<u8, (f32, f32)>,
    monitoring: bool,
    calibrating: bool,
    connecting: bool,
    section: Section,
    pane: Pane,
    /// The colour under the picker. Kept in OKLCH because that is what the
    /// pad and the hue slider work in; the device gets sRGB bytes.
    picker: Oklch,
    /// A drag moved the picker; apply it when the drag ends rather than on
    /// every frame, which would flood the keyboard.
    picker_dirty: bool,
    /// Whether the destructive actions are showing. Off every launch: a
    /// disclosure that remembers being open defeats the point of folding it.
    danger_open: bool,
    /// Effect to restore when the backlight is switched back on.
    last_mode: u8,
    /// A slider changed lighting; push it once the drag ends rather than on
    /// every frame, which would flood the device.
    led_dirty: bool,
    status: SharedString,
    error: Option<SharedString>,
}

impl ControlHost for SpawnApp {
    fn control_state(&self) -> &ControlState {
        &self.controls
    }

    fn control_state_mut(&mut self) -> &mut ControlState {
        &mut self.controls
    }

    fn track_dragged(&mut self, id: ComboId, at: Point<f32>, cx: &mut Context<Self>) {
        match id {
            "trigger" => {
                let v = round_step(value_at(MIN_TRIGGER, MAX_TRIGGER, at.x));
                self.edit_rt(|k| k.trigger_mm = v);
            }
            "press-rt" => {
                let ceiling = max_rt_for(self.rt_probe().axis_type);
                let v = round_step(value_at(MIN_RT, ceiling, at.x));
                self.edit_rt(|k| k.press_rt_mm = v);
            }
            "release-rt" => {
                let ceiling = max_rt_for(self.rt_probe().axis_type);
                let v = round_step(value_at(MIN_RT, ceiling, at.x));
                self.edit_rt(|k| k.release_rt_mm = v);
            }
            "brightness" => {
                let dragging = self.controls.is_dragging(id);
                self.led.brightness = stepped_level(dragging, at.x, self.led.brightness);
                self.led_dirty = true;
            }
            "speed" => {
                let dragging = self.controls.is_dragging(id);
                self.led.speed = stepped_level(dragging, at.x, self.led.speed);
                self.led_dirty = true;
            }
            "led-hue" => {
                self.picker = Oklch::new(
                    f64::from(at.x) * 360.0,
                    self.picker.chroma,
                    self.picker.lightness,
                );
                self.picker_dirty = true;
            }
            "led-pad" => {
                // Chroma runs across the pad, lightness up it.
                self.picker = Oklch::new(
                    self.picker.hue,
                    f64::from(at.x) * MAX_CHROMA,
                    f64::from(at.y),
                );
                self.picker_dirty = true;
            }
            "top-dead-zone" => {
                self.game_mode.top_dead_zone = round_step(value_at(0.0, MAX_DEAD_ZONE, at.x))
            }
            "bottom-dead-zone" => {
                self.game_mode.bottom_dead_zone = round_step(value_at(0.0, MAX_DEAD_ZONE, at.x))
            }
            _ => {}
        }
        // A drag is flushed when it ends. A key press has no end, so it is
        // applied here or it never reaches the keyboard at all.
        if !self.controls.is_dragging(id) {
            self.apply_pending(cx);
        }
        cx.notify();
    }

    /// A drag that ended over an occluding surface still has to apply what
    /// it changed. The default already pumps the toolkit's own drags.
    fn forwarded_mouse_up(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        vampir::end_drags(self, cx);
        self.apply_pending(cx);
    }
}

impl SpawnApp {
    pub fn new(window: &Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::with_worker(Worker::spawn(), cx);
        this.controls.theme.hue = HUE;
        // Keeps Scheme::System in step with the desktop, including when it
        // flips at sunset.
        this.controls.observe_appearance(window, cx);
        this
    }

    /// Build the view without touching real hardware.
    #[cfg(test)]
    pub fn offline(cx: &mut Context<Self>) -> Self {
        let mut this = Self::with_worker(Worker::offline(), cx);
        this.controls.theme.hue = HUE;
        this
    }

    fn with_worker(worker: Worker, cx: &mut Context<Self>) -> Self {
        worker.send(Cmd::Rescan);

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this.update(cx, |this, cx| this.drain(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            controls: ControlState::new(),
            worker,
            devices: Vec::new(),
            connected: None,
            info: None,
            game_mode: GameMode::default(),
            rt: vec![RtKey::default(); KEY_SLOTS],
            led: LedEffect::default(),
            custom_led: vec![DevRgb::default(); KEY_SLOTS],
            keys: vec![KeyAction::default(); KEY_SLOTS],
            selection: HashSet::new(),
            travel: HashMap::new(),
            monitoring: false,
            calibrating: false,
            connecting: false,
            picker: Oklch::new(150.0, MAX_CHROMA, 0.72),
            picker_dirty: false,
            danger_open: false,
            last_mode: 8,
            led_dirty: false,
            section: Section::Keyboard,
            pane: Pane::Actuation,
            status: "Searching for a keyboard".into(),
            error: None,
        }
    }

    // ----------------------------------------------------------------- drags

    /// Apply what a finished drag changed. Sliders move every frame, and
    /// writing the keyboard at that rate would flood it.
    fn apply_pending(&mut self, cx: &mut Context<Self>) {
        if self.picker_dirty {
            self.picker_dirty = false;
            let color = oklch_to_device(self.picker);
            self.choose_color(color);
        }
        if self.led_dirty {
            self.led_dirty = false;
            self.push_led();
        }
        cx.notify();
    }

    // ---------------------------------------------------------------- events

    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut dirty = false;
        while let Ok(evt) = self.worker.rx.try_recv() {
            dirty = true;
            match evt {
                Event::Devices(list) => {
                    if self.connected.is_none() {
                        self.status = if list.is_empty() {
                            "No keyboard found".into()
                        } else {
                            "Keyboard found".into()
                        };
                        // Connect on sight; hunting for a button is busywork.
                        if let Some(first) = list.first() {
                            if !self.connecting {
                                self.connecting = true;
                                self.status = "Connecting".into();
                                self.worker.send(Cmd::Connect(first.clone()));
                            }
                        }
                    }
                    self.devices = list;
                }
                Event::Connected(snap, label) => {
                    self.connecting = false;
                    self.apply_snapshot(*snap);
                    self.connected = Some(label);
                    self.status = "Connected".into();
                    self.error = None;
                }
                Event::Loaded(snap) => {
                    self.apply_snapshot(*snap);
                    self.status = "Reloaded from the keyboard".into();
                }
                Event::Disconnected => {
                    self.connected = None;
                    self.connecting = false;
                    self.info = None;
                    self.monitoring = false;
                    self.calibrating = false;
                    self.travel.clear();
                    self.status = "Released the keyboard".into();
                }
                Event::Telemetry(frames) => {
                    for f in frames {
                        let max = if f.max_stroke_mm() > 0.1 {
                            f.max_stroke_mm()
                        } else {
                            4.0
                        };
                        self.travel.insert(f.key_index, (f.stroke_mm(), max));
                    }
                }
                Event::Status(s) => self.status = s.into(),
                Event::Error(e) => {
                    self.connecting = false;
                    self.error = Some(e.into());
                }
            }
        }
        if dirty {
            cx.notify();
        }
    }

    fn apply_snapshot(&mut self, snap: crate::device::Snapshot) {
        self.info = Some(snap.info);
        self.game_mode = snap.game_mode;
        // A never-configured board reports zeros; show usable defaults so a
        // 0.00 mm actuation point can never be written back to it.
        self.rt = snap.rt.into_iter().map(RtKey::or_default).collect();
        self.led = snap.led;
        self.custom_led = snap.custom_led;
        self.keys = snap.keys;
        self.rt.resize(KEY_SLOTS, RtKey::default());
        self.keys.resize(KEY_SLOTS, KeyAction::default());
        self.custom_led.resize(KEY_SLOTS, DevRgb::default());
        if self.led.mode != LIGHTS_OFF {
            self.last_mode = self.led.mode;
        }
    }

    // -------------------------------------------------------------- selection

    fn board_slots() -> Vec<u8> {
        SPAWN_ROWS
            .iter()
            .flat_map(|r| r.iter())
            .map(|k| k.slot)
            .collect()
    }

    fn target_slots(&self) -> Vec<u8> {
        if self.selection.is_empty() {
            Self::board_slots()
        } else {
            let mut v: Vec<u8> = self.selection.iter().copied().collect();
            v.sort_unstable();
            v
        }
    }

    fn edit_rt(&mut self, f: impl Fn(&mut RtKey)) {
        for slot in self.target_slots() {
            if let Some(k) = self.rt.get_mut(slot as usize) {
                f(k);
            }
        }
    }

    fn rt_probe(&self) -> RtKey {
        self.target_slots()
            .first()
            .and_then(|s| self.rt.get(*s as usize))
            .copied()
            .unwrap_or_default()
    }

    /// Format a per-key value, saying so when the selection disagrees rather
    /// than silently showing whichever key happens to be first.
    fn readout_mm(&self, get: impl Fn(&RtKey) -> f32) -> String {
        let slots = self.target_slots();
        let mut iter = slots
            .iter()
            .filter_map(|s| self.rt.get(*s as usize))
            .map(&get);
        let Some(first) = iter.next() else {
            return "—".into();
        };
        if iter.any(|v| (v - first).abs() > f32::EPSILON) {
            "Mixed".into()
        } else {
            format!("{first:.2} mm")
        }
    }

    /// Switching view takes the focused element with it, and GPUI dispatches
    /// nothing from a handle that has left the tree.
    fn show_section(&mut self, section: Section, _window: &mut Window, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    fn view_menu(&self) -> Vec<MenuItem> {
        vec![
            MenuItem::header("View"),
            MenuItem::action("keyboard", "Keyboard").checked(self.section == Section::Keyboard),
            MenuItem::action("settings", "Settings").checked(self.section == Section::Settings),
            MenuItem::separator(),
            device_item("reload", "Reload from keyboard", self.is_connected()),
            device_item("release", "Release keyboard", self.is_connected()),
        ]
    }

    fn is_connected(&self) -> bool {
        self.connected.is_some()
    }

    fn require_device(&mut self) -> bool {
        if self.is_connected() {
            true
        } else {
            self.error = Some("No keyboard connected".into());
            false
        }
    }

    fn push_led(&mut self) {
        // A board can come back holding a level outside the range the
        // firmware actually honours; never write one of those back.
        self.led.brightness = self.led.brightness.clamp(MIN_LEVEL as u8, MAX_LEVEL as u8);
        self.led.speed = self.led.speed.clamp(MIN_LEVEL as u8, MAX_LEVEL as u8);
        if self.is_connected() {
            self.worker.send(Cmd::ApplyLed(self.led));
        } else {
            self.error = Some("No keyboard connected".into());
        }
    }

    fn paint(&mut self, color: DevRgb) {
        for slot in self.target_slots() {
            if let Some(s) = self.custom_led.get_mut(slot as usize) {
                *s = color;
            }
        }
        if self.is_connected() {
            self.worker
                .send(Cmd::ApplyCustomLed(self.custom_led.clone()));
        } else {
            self.error = Some("No keyboard connected".into());
        }
    }

    /// Apply a chosen colour, wherever the current mode says it belongs.
    fn choose_color(&mut self, color: DevRgb) {
        if self.led.mode == CUSTOM_LIGHTING {
            self.paint(color);
        } else {
            // The firmware ignores the RGB unless it is told to use it.
            self.led.color_mode = COLOR_CUSTOM;
            self.led.primary = color;
            self.push_led();
        }
    }

    fn assign(&mut self, action: KeyAction) {
        if self.selection.is_empty() {
            self.error = Some("Select a key first".into());
            return;
        }
        self.error = None;
        for slot in self.target_slots() {
            if let Some(k) = self.keys.get_mut(slot as usize) {
                *k = action;
            }
        }
    }

    #[cfg(test)]
    pub fn set_section(&mut self, section: Section) {
        self.section = section;
    }

    #[cfg(test)]
    pub fn set_pane(&mut self, pane: Pane) {
        self.pane = pane;
    }

    #[cfg(test)]
    pub fn select_all(&mut self) {
        self.selection = Self::board_slots().into_iter().collect();
    }

    #[cfg(test)]
    pub fn nudge_trigger(&mut self, delta: f32) {
        self.edit_rt(|k| k.trigger_mm = (k.trigger_mm + delta).clamp(MIN_TRIGGER, MAX_TRIGGER));
    }

    #[cfg(test)]
    pub fn probe_trigger_mm(&self) -> f32 {
        self.rt_probe().trigger_mm
    }
}

// ------------------------------------------------------------------- views

/// A labelled row: text on the left, control on the right. Sections group
/// these; the app deliberately avoids nesting cards inside cards.
fn field(palette: Palette, label: &str, control: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .py(px(4.0))
        .child(
            div()
                .text_size(px(TEXT_SIZE))
                .text_color(palette.text_primary)
                .child(label.to_string()),
        )
        .child(control)
        .into_any_element()
}

fn value_text(palette: Palette, text: impl Into<SharedString>) -> AnyElement {
    div()
        .min_w(px(70.0))
        .text_size(px(TEXT_SIZE))
        .text_color(palette.text_secondary)
        .child(text.into())
        .into_any_element()
}

fn note(palette: Palette, text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_size(px(SMALL_TEXT_SIZE))
        .text_color(palette.text_secondary)
        .child(text.into())
        .into_any_element()
}

impl SpawnApp {
    fn section(&self, title: &str, body: AnyElement) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(caption(self.controls.palette(), title))
            .child(body)
            .into_any_element()
    }

    fn render_board(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.controls.palette();
        let dark = palette.is_dark;
        let selection = self.selection.clone();
        let travel = self.travel.clone();
        let live = self.monitoring || self.calibrating;
        let pane = self.pane;
        let show_paint = pane == Pane::Lighting && self.led.mode == CUSTOM_LIGHTING;
        let custom = self.custom_led.clone();
        let rt = self.rt.clone();
        let keys = self.keys.clone();
        let effect_color = (self.led.color_mode == COLOR_CUSTOM).then_some(self.led.primary);

        let mut rows: Vec<AnyElement> = Vec::new();
        for row in SPAWN_ROWS {
            let mut caps: Vec<AnyElement> = Vec::new();
            for key in row.iter() {
                let cx: &mut Context<Self> = &mut *cx;
                let k: &KeyCap = key;
                let w = UNIT * k.width + GAP * (k.width - 1.0);
                let selected = selection.contains(&k.slot);
                let slot = k.slot;

                // The board is the one view of the keyboard, so it shows
                // whatever the pane under it is about. Live travel outranks
                // all of it: while the monitor runs, that is what a cap means.
                let face = key_face(palette);
                let base = if live {
                    let (mm, max) = travel.get(&slot).copied().unwrap_or((0.0, 4.0));
                    travel_color(palette, mm / max.max(0.1))
                } else if show_paint {
                    let c = custom.get(slot as usize).copied().unwrap_or_default();
                    if c.r == 0 && c.g == 0 && c.b == 0 {
                        face
                    } else {
                        device_color(c)
                    }
                } else {
                    match pane {
                        // Deeper actuation reads as a darker cap, so the
                        // shape of a per-key profile is visible at a glance.
                        Pane::Actuation => {
                            let k = rt.get(slot as usize).copied().unwrap_or_default();
                            let depth =
                                ratio(k.trigger_mm, MIN_TRIGGER, MAX_TRIGGER).clamp(0.0, 1.0);
                            shade(face, -0.22 * depth)
                        }
                        // A remapped key stands out from the ones still doing
                        // what their legend says.
                        Pane::Keys => {
                            let remapped = keys
                                .get(slot as usize)
                                .is_some_and(|a| a.page != Page::Default);
                            if remapped { shade(face, 0.10) } else { face }
                        }
                        Pane::Lighting => effect_color.map_or(face, device_color),
                    }
                };

                // In the Keys pane a cap says what it does now, not what is
                // printed on it.
                let label: SharedString = match pane {
                    Pane::Keys => keys
                        .get(slot as usize)
                        .and_then(|a| assigned_label(a))
                        .map_or_else(|| k.label.into(), SharedString::from),
                    _ => k.label.into(),
                };

                // Selection crosses over rather than snapping. Read during
                // render, so a key selected by "Select all" travels exactly
                // as one clicked with the pointer does.
                let id: ElementId = ("key", slot as usize).into();
                let on = self.controls.blend(&id, selected, SWITCH_SLIDE);

                caps.push(
                    div()
                        .id(("key", slot as usize))
                        .w(px(w))
                        .h(px(UNIT))
                        .flex()
                        .items_center()
                        .justify_center()
                        // A keycap is something you press, so it is raised.
                        .rounded(px(5.0))
                        .bg(lit_mix(base, palette.accent, 0.10, on))
                        .border_1()
                        .border_color(lerp(rim(base, dark), palette.accent, on))
                        .shadow(raised(dark))
                        .text_size(px(SMALL_TEXT_SIZE))
                        .text_color(lerp(palette.control_label, palette.primary_label, on))
                        .cursor_pointer()
                        .child(label)
                        // Select on release: a press is also how a drag starts.
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                if !this.selection.remove(&slot) {
                                    this.selection.insert(slot);
                                }
                                cx.notify();
                            }),
                        )
                        .into_any_element(),
                );
            }
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(GAP))
                    .children(caps)
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(GAP))
            .p(px(BOARD_PAD))
            .rounded(px(10.0))
            .bg(lit(palette.field_surface, 0.02))
            .border_1()
            .border_color(palette.field_border)
            .children(rows)
            .into_any_element()
    }
}

// -------------------------------------------------------------- tab bodies

impl SpawnApp {
    fn render_actuation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.controls.palette();
        let probe = self.rt_probe();
        // The switch under the key sets how far rapid trigger can be pushed.
        let rt_ceiling = max_rt_for(probe.axis_type);

        let trigger_text = self.readout_mm(|k| k.trigger_mm);
        let press_text = self.readout_mm(|k| k.press_rt_mm);
        let release_text = self.readout_mm(|k| k.release_rt_mm);

        let trigger = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .w(px(320.0))
            .child(value_text(palette, trigger_text))
            .child(slider(
                "trigger",
                ratio(probe.trigger_mm, MIN_TRIGGER, MAX_TRIGGER),
                SliderTrack::Continuous,
                palette,
                self,
                cx,
            ))
            .into_any_element();

        let press = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .w(px(320.0))
            .child(value_text(palette, press_text))
            .child(slider(
                "press-rt",
                ratio(probe.press_rt_mm, MIN_RT, rt_ceiling),
                SliderTrack::Continuous,
                palette,
                self,
                cx,
            ))
            .into_any_element();

        let release = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .w(px(320.0))
            .child(value_text(palette, release_text))
            .child(slider(
                "release-rt",
                ratio(probe.release_rt_mm, MIN_RT, rt_ceiling),
                SliderTrack::Continuous,
                palette,
                self,
                cx,
            ))
            .into_any_element();

        let axis_names: Vec<String> = AXIS_PROFILES
            .iter()
            .map(|(_, n, _)| (*n).to_string())
            .collect();
        let axis_index = AXIS_PROFILES
            .iter()
            .position(|(v, _, _)| *v == probe.axis_type)
            .unwrap_or(0);

        let continuous = switch(
            "whole-fast",
            probe.whole_fast,
            None,
            true,
            palette,
            self,
            cx,
            |this, on, _w, cx| {
                this.edit_rt(|k| k.whole_fast = on);
                cx.notify();
            },
        )
        .into_any_element();

        let rampage = switch(
            "rampage",
            probe.rampage,
            None,
            true,
            palette,
            self,
            cx,
            |this, on, _w, cx| {
                this.edit_rt(|k| k.rampage = on);
                cx.notify();
            },
        )
        .into_any_element();

        let mut presets: Vec<AnyElement> = Vec::new();
        for (name, value) in [
            ("Hair", 0.3f32),
            ("Fast", 0.8),
            ("Normal", 1.5),
            ("Deep", 2.2),
        ] {
            let cx: &mut Context<Self> = &mut *cx;
            presets.push(
                button(
                    SharedString::from(format!("preset-{name}")),
                    name,
                    ButtonVariant::Soft,
                    true,
                    palette,
                    cx,
                    move |this, _w, cx| {
                        this.edit_rt(|k| k.trigger_mm = value);
                        cx.notify();
                    },
                )
                .into_any_element(),
            );
        }

        let monitoring = self.monitoring;

        div()
            .flex()
            .flex_col()
            .gap(px(22.0))
            .child(
                self.section(
                    "Actuation point",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(trigger)
                        .child(div().flex().flex_row().gap(px(6.0)).children(presets))
                        .into_any_element(),
                ),
            )
            .child(separator(false, palette))
            .child(
                self.section(
                    "Rapid trigger",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(note(
                            palette,
                            "Re-arms a key as soon as it reverses, instead of waiting for it to \
                         pass the actuation point.",
                        ))
                        .child(field(palette, "Press", press))
                        .child(field(palette, "Release", release))
                        .child(field(palette, "Continuous", continuous))
                        .child(field(palette, "Rampage", rampage))
                        .into_any_element(),
                ),
            )
            .child(separator(false, palette))
            .child(
                self.section(
                    "Switch profile",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(note(
                            palette,
                            "Which magnetic switch is fitted under the selected keys. The \
                             board maps travel differently for each, so the wrong one makes \
                             every depth read short or long.",
                        ))
                        .child(combo(
                            "axis",
                            axis_index,
                            &axis_names,
                            Some(260.0),
                            palette,
                            self,
                            cx,
                            |this, index, _w, cx| {
                                if let Some((value, _, _)) = AXIS_PROFILES.get(index) {
                                    let value = *value;
                                    this.edit_rt(|k| k.axis_type = value);
                                }
                                cx.notify();
                            },
                        ))
                        .into_any_element(),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .child(button(
                        "apply-rt",
                        "Apply to keyboard",
                        ButtonVariant::Primary,
                        true,
                        palette,
                        cx,
                        |this, _w, cx| {
                            if this.require_device() {
                                this.worker.send(Cmd::ApplyRt(this.rt.clone()));
                                this.status = "Writing actuation".into();
                            }
                            cx.notify();
                        },
                    ))
                    .child(button(
                        "monitor",
                        if monitoring {
                            "Stop live view"
                        } else {
                            "Live view"
                        },
                        if monitoring {
                            ButtonVariant::Danger
                        } else {
                            ButtonVariant::Soft
                        },
                        true,
                        palette,
                        cx,
                        |this, _w, cx| {
                            if this.require_device() {
                                this.monitoring = !this.monitoring;
                                this.travel.clear();
                                this.worker.send(Cmd::SetMonitor(this.monitoring));
                            }
                            cx.notify();
                        },
                    )),
            )
            .into_any_element()
    }

    fn render_lighting(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.controls.palette();
        let led = self.led;
        let caps = LED_MODES
            .iter()
            .find(|m| m.value == led.mode)
            .unwrap_or(&LED_MODES[0]);
        let (show_speed, show_color) = (caps.speed, caps.color);
        let directions = directions_for(led.mode);
        let per_key = led.mode == CUSTOM_LIGHTING;
        let auto_color = led.color_mode == COLOR_AUTO;
        let on = led.mode != LIGHTS_OFF;

        let backlight = switch(
            "backlight",
            on,
            None,
            true,
            palette,
            self,
            cx,
            |this, want_on, _w, cx| {
                // Mode 0 is the firmware's own "off".
                this.led.mode = if want_on { this.last_mode } else { LIGHTS_OFF };
                this.push_led();
                cx.notify();
            },
        )
        .into_any_element();

        // Chips, not buttons: twenty full-width bars is a wall to scroll
        // past, where a wrapping field of labels can be read at a glance.
        let mut effects: Vec<AnyElement> = Vec::new();
        for mode in LED_MODES {
            let cx: &mut Context<Self> = &mut *cx;
            let value = mode.value;
            effects.push(
                chip(
                    SharedString::from(format!("led-{value}")),
                    mode.name,
                    led.mode == value,
                    true,
                    palette,
                    self,
                    cx,
                    move |this, _w, cx| {
                        this.led.mode = value;
                        this.last_mode = value;
                        this.push_led();
                        cx.notify();
                    },
                )
                .into_any_element(),
            );
        }

        let brightness = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .w(px(300.0))
            .child(value_text(
                palette,
                format!("{} of 5", led.brightness.max(1)),
            ))
            .child(slider(
                "brightness",
                ratio(led.brightness as f32, MIN_LEVEL, MAX_LEVEL),
                SliderTrack::Stepped { stops: 5 },
                palette,
                self,
                cx,
            ))
            .into_any_element();

        let speed = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .w(px(300.0))
            .child(value_text(
                palette,
                format!("{} of 5", led.speed.clamp(1, 5)),
            ))
            .child(slider(
                "speed",
                ratio(led.speed as f32, MIN_LEVEL, MAX_LEVEL),
                SliderTrack::Stepped { stops: 5 },
                palette,
                self,
                cx,
            ))
            .into_any_element();

        let mut swatches: Vec<AnyElement> = Vec::new();
        for (index, (name, color)) in SWATCHES.into_iter().enumerate() {
            let cx: &mut Context<Self> = &mut *cx;
            let active = if per_key {
                self.target_slots()
                    .first()
                    .and_then(|s| self.custom_led.get(*s as usize))
                    .copied()
                    == Some(color)
            } else {
                !auto_color && led.primary == color
            };
            let fill = device_color(color);
            let id: ElementId = ("swatch", index).into();
            let on = self.controls.blend(&id, active, SWITCH_SLIDE);
            swatches.push(
                div()
                    .id(("swatch", index))
                    .w(px(26.0))
                    .h(px(26.0))
                    .rounded(px(5.0))
                    .bg(lit(fill, 0.10))
                    .border_2()
                    .border_color(lerp(rim(fill, palette.is_dark), palette.accent, on))
                    .shadow(raised(palette.is_dark))
                    .cursor_pointer()
                    .tooltip(Tooltip::text(name, palette))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.choose_color(color);
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
            );
        }

        let rainbow = switch(
            "auto-color",
            auto_color,
            None,
            !per_key,
            palette,
            self,
            cx,
            |this, want_auto, _w, cx| {
                this.led.color_mode = if want_auto { COLOR_AUTO } else { COLOR_CUSTOM };
                this.push_led();
                cx.notify();
            },
        )
        .into_any_element();

        let mut levels = div().flex().flex_col().gap(px(2.0));
        levels = levels.child(field(palette, "Brightness", brightness));

        if show_speed {
            levels = levels.child(field(palette, "Speed", speed));
        }
        if let Some(pair) = directions {
            let names: Vec<String> = pair.iter().map(|(_, n)| (*n).to_string()).collect();
            let selected = pair
                .iter()
                .position(|(v, _)| *v == led.direction)
                .unwrap_or(0);
            levels = levels.child(field(
                palette,
                "Direction",
                segmented(
                    "direction",
                    &names,
                    selected,
                    true,
                    palette,
                    self,
                    cx,
                    move |this, index, _w, cx| {
                        if let Some((value, _)) = pair.get(index) {
                            this.led.direction = *value;
                            this.push_led();
                        }
                        cx.notify();
                    },
                )
                .into_any_element(),
            ));
        }

        let mut colour = div().flex().flex_col().gap(px(8.0));
        if !per_key {
            colour = colour.child(field(palette, "Let the keyboard choose", rainbow));
        }
        let picker = self.picker;
        let picker_preview = picker.to_rgba();
        // Into an owned element straight away: the returned value borrows
        // `cx`, and both controls need it.
        let pad = color_pad("led-pad", picker, 96.0, palette, self, cx).into_any_element();
        let hue = hue_slider("led-hue", picker.hue, palette, self, cx).into_any_element();

        colour = colour
            .child(note(
                palette,
                if per_key {
                    "Colours the selected keys."
                } else if auto_color {
                    "The keyboard is cycling its own colours. Turn that off to pick one."
                } else {
                    "Applies to the whole effect."
                },
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(14.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_wrap()
                                    .gap(px(6.0))
                                    .children(swatches),
                            )
                            .child(caption(palette, "Or mix one")),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .w(px(240.0))
                            .child(pad)
                            .child(hue)
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .w(px(22.0))
                                            .h(px(22.0))
                                            .rounded(px(5.0))
                                            .bg(lit(picker_preview, 0.10))
                                            .border_1()
                                            .border_color(rim(picker_preview, palette.is_dark))
                                            .shadow(raised(palette.is_dark)),
                                    )
                                    .child({
                                        let c = oklch_to_device(picker);
                                        note(palette, format!("{:02X}{:02X}{:02X}", c.r, c.g, c.b))
                                    }),
                            ),
                    ),
            );

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(22.0))
            .child(
                self.section(
                    "Backlight",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(field(palette, "On", backlight))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap(px(6.0))
                                .children(effects),
                        )
                        .into_any_element(),
                ),
            )
            .child(separator(false, palette))
            .child(self.section("Levels", levels.into_any_element()));

        if show_color {
            body = body
                .child(separator(false, palette))
                .child(self.section("Colour", colour.into_any_element()));
        }

        body.into_any_element()
    }
}

/// A colour the keyboard reports or accepts, as the toolkit sees it.
fn device_color(c: DevRgb) -> gpui::Rgba {
    vampir::color::rgba(
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        1.0,
    )
}

/// The colours the picker offers, as name and value.
///
/// Saturated primaries and secondaries, not tinted ones: these drive LEDs,
/// where a mint like `#3CE66E` reads as teal rather than as green. Every
/// entry sits on a channel extreme so what is asked for is what lights up.
const SWATCHES: [(&str, DevRgb); 10] = [
    ("White", DevRgb::new(255, 255, 255)),
    ("Red", DevRgb::new(255, 0, 0)),
    ("Orange", DevRgb::new(255, 80, 0)),
    ("Yellow", DevRgb::new(255, 255, 0)),
    ("Green", DevRgb::new(0, 255, 0)),
    ("Cyan", DevRgb::new(0, 255, 255)),
    ("Blue", DevRgb::new(0, 0, 255)),
    ("Purple", DevRgb::new(128, 0, 255)),
    ("Magenta", DevRgb::new(255, 0, 255)),
    ("Off", DevRgb::new(0, 0, 0)),
];

/// An OKLCH colour as the bytes the keyboard takes.
fn oklch_to_device(color: Oklch) -> DevRgb {
    let [r, g, b, _] = channels(color.to_rgba());
    let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    DevRgb::new(to_byte(r), to_byte(g), to_byte(b))
}

// -------------------------------------------------------------- keys tab

/// Assignments offered in the Keys tab. Usage IDs are from the HID
/// Keyboard/Keypad page; media entries use the Consumer page.
const ASSIGNABLE: &[(&str, &[(&str, KeyAction)])] = &[
    (
        "Modifiers",
        &[
            ("Left Ctrl", KeyAction::keyboard(0, 0xE0)),
            ("Left Shift", KeyAction::keyboard(0, 0xE1)),
            ("Left Alt", KeyAction::keyboard(0, 0xE2)),
            ("Left Win", KeyAction::keyboard(0, 0xE3)),
            ("Right Ctrl", KeyAction::keyboard(0, 0xE4)),
            ("Right Shift", KeyAction::keyboard(0, 0xE5)),
            ("Right Alt", KeyAction::keyboard(0, 0xE6)),
        ],
    ),
    (
        "Editing",
        &[
            ("Esc", KeyAction::keyboard(0, 0x29)),
            ("Tab", KeyAction::keyboard(0, 0x2B)),
            ("Enter", KeyAction::keyboard(0, 0x28)),
            ("Backspace", KeyAction::keyboard(0, 0x2A)),
            ("Delete", KeyAction::keyboard(0, 0x4C)),
            ("Insert", KeyAction::keyboard(0, 0x49)),
            ("Home", KeyAction::keyboard(0, 0x4A)),
            ("End", KeyAction::keyboard(0, 0x4D)),
            ("Page up", KeyAction::keyboard(0, 0x4B)),
            ("Page down", KeyAction::keyboard(0, 0x4E)),
            ("Caps lock", KeyAction::keyboard(0, 0x39)),
        ],
    ),
    (
        "Arrows",
        &[
            ("Left", KeyAction::keyboard(0, 0x50)),
            ("Down", KeyAction::keyboard(0, 0x51)),
            ("Up", KeyAction::keyboard(0, 0x52)),
            ("Right", KeyAction::keyboard(0, 0x4F)),
        ],
    ),
    (
        "Function",
        &[
            ("F1", KeyAction::keyboard(0, 0x3A)),
            ("F2", KeyAction::keyboard(0, 0x3B)),
            ("F3", KeyAction::keyboard(0, 0x3C)),
            ("F4", KeyAction::keyboard(0, 0x3D)),
            ("F5", KeyAction::keyboard(0, 0x3E)),
            ("F6", KeyAction::keyboard(0, 0x3F)),
            ("F7", KeyAction::keyboard(0, 0x40)),
            ("F8", KeyAction::keyboard(0, 0x41)),
            ("F9", KeyAction::keyboard(0, 0x42)),
            ("F10", KeyAction::keyboard(0, 0x43)),
            ("F11", KeyAction::keyboard(0, 0x44)),
            ("F12", KeyAction::keyboard(0, 0x45)),
        ],
    ),
    (
        "Mouse",
        &[
            ("Left click", KeyAction::mouse(1, 1)),
            ("Right click", KeyAction::mouse(1, 2)),
            ("Middle click", KeyAction::mouse(1, 4)),
            ("Back", KeyAction::mouse(1, 8)),
            ("Forward", KeyAction::mouse(1, 16)),
            // Scrolling is the same page under a different function byte:
            // 1 is a button, 3 is the wheel, and down travels as -1.
            ("Scroll up", KeyAction::mouse(3, 1)),
            ("Scroll down", KeyAction::mouse(3, 255)),
        ],
    ),
];

fn media_actions() -> Vec<(&'static str, KeyAction)> {
    vec![
        ("Play or pause", KeyAction::consumer(0x00CD)),
        ("Next track", KeyAction::consumer(0x00B5)),
        ("Previous track", KeyAction::consumer(0x00B6)),
        ("Stop", KeyAction::consumer(0x00B7)),
        ("Mute", KeyAction::consumer(0x00E2)),
        ("Volume up", KeyAction::consumer(0x00E9)),
        ("Volume down", KeyAction::consumer(0x00EA)),
    ]
}

/// The short name of an assignment, for a keycap. `None` when the key still
/// does what its legend says.
fn assigned_label(action: &KeyAction) -> Option<&'static str> {
    if action.page == Page::Default {
        return None;
    }
    ASSIGNABLE
        .iter()
        .flat_map(|(_, list)| list.iter())
        .find(|(_, a)| *a == *action)
        .map(|(name, _)| *name)
        .or_else(|| {
            media_actions()
                .into_iter()
                .find(|(_, a)| *a == *action)
                .map(|(name, _)| name)
        })
}

/// Human-readable summary of what a slot currently does.
fn describe(action: &KeyAction, slot: u8) -> String {
    match action.page {
        Page::Default => key_by_slot(slot)
            .map(|k| format!("default ({})", k.label))
            .unwrap_or_else(|| "default".into()),
        Page::Keyboard => ASSIGNABLE
            .iter()
            .flat_map(|(_, l)| l.iter())
            .find(|(_, a)| a.page == Page::Keyboard && a.p2 == action.p2)
            .map(|(n, _)| n.to_string())
            .unwrap_or_else(|| format!("usage {:#04x}", action.p2)),
        Page::Mouse => format!("mouse button {}", action.p2),
        Page::ConsumerKey => {
            let usage = u16::from(action.p1) | (u16::from(action.p2) << 8);
            media_actions()
                .into_iter()
                .find(|(_, a)| u16::from(a.p1) | (u16::from(a.p2) << 8) == usage)
                .map(|(n, _)| n.to_string())
                .unwrap_or_else(|| format!("consumer {usage:#06x}"))
        }
        Page::Macro => format!("macro {}", action.p1),
        Page::Dks => format!("DKS {}", action.p1),
        Page::Mt => "mod-tap".into(),
        Page::Tgl => "toggle".into(),
        Page::Socd => "SOCD".into(),
        Page::Rs => "rapid switch".into(),
        Page::Cb => "combo".into(),
        Page::Func => "function".into(),
        Page::SystemKey => "system".into(),
        Page::ExtraFunction => "extra".into(),
    }
}

impl SpawnApp {
    fn render_keys(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.controls.palette();

        let current: Vec<AnyElement> = if self.selection.is_empty() {
            vec![note(palette, "Select a key on the board to remap it.")]
        } else {
            self.target_slots()
                .into_iter()
                .take(6)
                .map(|s| {
                    let a = self.keys.get(s as usize).copied().unwrap_or_default();
                    note(
                        palette,
                        format!(
                            "{} is {}",
                            key_by_slot(s).map(|k| k.label).unwrap_or("?"),
                            describe(&a, s)
                        ),
                    )
                })
                .collect()
        };

        // What the first targeted key does now, so each group can show the
        // assignment rather than making the user remember it.
        let assigned = self
            .target_slots()
            .first()
            .and_then(|s| self.keys.get(*s as usize))
            .copied()
            .unwrap_or_default();

        let mut groups: Vec<AnyElement> = Vec::new();
        for (group, list) in ASSIGNABLE {
            let labels: Vec<String> = list.iter().map(|(n, _)| (*n).to_string()).collect();
            let selected = list.iter().position(|(_, a)| *a == assigned);
            let actions: Vec<KeyAction> = list.iter().map(|(_, a)| *a).collect();
            let cx: &mut Context<Self> = &mut *cx;
            groups.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(caption(palette, group))
                    // Chips, not buttons: these are sets of choices, and a
                    // wrapping field of labels reads at a glance where a
                    // column of full-width bars does not.
                    .child(chip_group(
                        group,
                        &labels,
                        ChipSelection::One(selected),
                        true,
                        palette,
                        self,
                        cx,
                        move |this, index, _w, cx| {
                            if let Some(action) = actions.get(index) {
                                this.assign(*action);
                            }
                            cx.notify();
                        },
                    ))
                    .into_any_element(),
            );
        }

        let media = media_actions();
        let media_labels: Vec<String> = media.iter().map(|(n, _)| (*n).to_string()).collect();
        let media_selected = media.iter().position(|(_, a)| *a == assigned);
        let media_actions_list: Vec<KeyAction> = media.iter().map(|(_, a)| *a).collect();
        groups.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(caption(palette, "Media"))
                .child(chip_group(
                    "media",
                    &media_labels,
                    ChipSelection::One(media_selected),
                    true,
                    palette,
                    self,
                    cx,
                    move |this, index, _w, cx| {
                        if let Some(action) = media_actions_list.get(index) {
                            this.assign(*action);
                        }
                        cx.notify();
                    },
                ))
                .into_any_element(),
        );

        div()
            .flex()
            .flex_col()
            .gap(px(22.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .children(current)
                    .into_any_element(),
            )
            .child(separator(false, palette))
            .child(div().flex().flex_col().gap(px(18.0)).children(groups))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .child(button(
                        "apply-keys",
                        "Apply keymap",
                        ButtonVariant::Primary,
                        true,
                        palette,
                        cx,
                        |this, _w, cx| {
                            if this.require_device() {
                                this.worker.send(Cmd::ApplyKeys(this.keys.clone()));
                                this.status = "Writing keymap".into();
                            }
                            cx.notify();
                        },
                    ))
                    .child(button(
                        "clear-keys",
                        "Restore default",
                        ButtonVariant::Soft,
                        true,
                        palette,
                        cx,
                        |this, _w, cx| {
                            this.assign(KeyAction::disabled());
                            cx.notify();
                        },
                    )),
            )
            .into_any_element()
    }

    fn render_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.controls.palette();
        let gm = self.game_mode.clone();
        let danger_open = self.danger_open;
        let calibrating = self.calibrating;

        let rate_names: Vec<String> = REPORT_RATES.iter().map(|(_, n)| (*n).to_string()).collect();
        let rate_index = REPORT_RATES
            .iter()
            .position(|(code, _)| *code == gm.report_rate)
            .unwrap_or(0);

        let top = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .w(px(300.0))
            .child(value_text(palette, format!("{:.2} mm", gm.top_dead_zone)))
            .child(slider(
                "top-dead-zone",
                ratio(gm.top_dead_zone, 0.0, MAX_DEAD_ZONE),
                SliderTrack::Continuous,
                palette,
                self,
                cx,
            ))
            .into_any_element();

        let bottom = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .w(px(300.0))
            .child(value_text(
                palette,
                format!("{:.2} mm", gm.bottom_dead_zone),
            ))
            .child(slider(
                "bottom-dead-zone",
                ratio(gm.bottom_dead_zone, 0.0, MAX_DEAD_ZONE),
                SliderTrack::Continuous,
                palette,
                self,
                cx,
            ))
            .into_any_element();

        let stability = switch(
            "stability",
            gm.stability_mode != 0,
            None,
            true,
            palette,
            self,
            cx,
            |this, on, _w, cx| {
                this.game_mode.stability_mode = u8::from(on);
                cx.notify();
            },
        )
        .into_any_element();

        let autocal = switch(
            "autocal",
            gm.auto_calibration != 0,
            None,
            true,
            palette,
            self,
            cx,
            |this, on, _w, cx| {
                this.game_mode.auto_calibration = u8::from(on);
                cx.notify();
            },
        )
        .into_any_element();

        let mut resets: Vec<AnyElement> = Vec::new();
        for (id, label, scope) in [
            ("reset-keys", "Keymap", ResetScope::Keys),
            ("reset-light", "Lighting", ResetScope::Lighting),
            ("reset-cal", "Calibration", ResetScope::Calibration),
            ("reset-all", "Everything", ResetScope::All),
        ] {
            let cx: &mut Context<Self> = &mut *cx;
            resets.push(
                button(
                    id,
                    label,
                    ButtonVariant::Danger,
                    true,
                    palette,
                    cx,
                    move |this, _w, cx| {
                        if this.require_device() {
                            this.worker.send(Cmd::FactoryReset(scope));
                        }
                        cx.notify();
                    },
                )
                .into_any_element(),
            );
        }

        let device = match &self.info {
            Some(i) => div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(field(
                    palette,
                    "Firmware",
                    value_text(palette, i.version_string()),
                ))
                .child(field(
                    palette,
                    "USB identifier",
                    value_text(palette, format!("{:04X}:{:04X}", i.vid, i.pid)),
                ))
                .child(field(
                    palette,
                    "Travel resolution",
                    value_text(
                        palette,
                        if i.rt_precision == 1 {
                            "0.001 mm"
                        } else {
                            "0.01 mm"
                        },
                    ),
                ))
                .into_any_element(),
            None => note(palette, "Connect a keyboard to see its details."),
        };

        div()
            .flex()
            .flex_col()
            .gap(px(22.0))
            .child(self.section("Device", device))
            .child(separator(false, palette))
            .child(
                self.section(
                    "Polling rate",
                    segmented(
                        "rate",
                        &rate_names,
                        rate_index,
                        true,
                        palette,
                        self,
                        cx,
                        |this, index, _w, cx| {
                            if let Some((code, _)) = REPORT_RATES.get(index) {
                                this.game_mode.report_rate = *code;
                            }
                            cx.notify();
                        },
                    )
                    .into_any_element(),
                ),
            )
            .child(separator(false, palette))
            .child(
                self.section(
                    "Dead zones",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(note(
                            palette,
                            "Travel ignored at the top and bottom of the stroke.",
                        ))
                        .child(field(palette, "Top", top))
                        .child(field(palette, "Bottom", bottom))
                        .into_any_element(),
                ),
            )
            .child(separator(false, palette))
            .child(
                self.section(
                    "Behaviour",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(field(palette, "Stability mode", stability))
                        .child(field(palette, "Automatic calibration", autocal))
                        .into_any_element(),
                ),
            )
            .child(button(
                "apply-settings",
                "Apply settings",
                ButtonVariant::Primary,
                true,
                palette,
                cx,
                |this, _w, cx| {
                    if this.require_device() {
                        this.worker.send(Cmd::ApplyGameMode(this.game_mode.clone()));
                    }
                    cx.notify();
                },
            ))
            .child(separator(false, palette))
            .child(
                self.section(
                    "Calibration",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(note(
                            palette,
                            "Press every key fully to the bottom while this runs, then stop.",
                        ))
                        .child(button(
                            "calib",
                            if calibrating {
                                "Finish calibration"
                            } else {
                                "Start calibration"
                            },
                            if calibrating {
                                ButtonVariant::Danger
                            } else {
                                ButtonVariant::Soft
                            },
                            true,
                            palette,
                            cx,
                            |this, _w, cx| {
                                if this.require_device() {
                                    this.calibrating = !this.calibrating;
                                    this.travel.clear();
                                    this.worker.send(Cmd::SetCalibration(this.calibrating));
                                }
                                cx.notify();
                            },
                        ))
                        .into_any_element(),
                ),
            )
            .child(separator(false, palette))
            .child(self.section("Colour scheme", scheme_picker("scheme", palette, self, cx)))
            .child(separator(false, palette))
            // Folded away by default: none of it can be undone, and it is not
            // what anyone opens Settings to do.
            .child(collapsible(
                "danger-zone",
                "Danger zone",
                danger_open,
                palette,
                self,
                cx,
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .pt(px(4.0))
                    .child(note(
                        palette,
                        "Each of these restores the firmware's own defaults and cannot be \
                         undone.",
                    ))
                    .child(div().flex().flex_row().gap(px(6.0)).children(resets)),
                |this, open, _w, cx| {
                    this.danger_open = open;
                    cx.notify();
                },
            ))
            .into_any_element()
    }
}

impl Render for SpawnApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The toolkit owns the scheme now: it tracks the desktop through
        // `observe_appearance` and cross-fades the palette itself.
        let palette = self.controls.palette();
        let section = self.section;
        let pane = self.pane;

        let pane_tabs: Vec<VTab> = Pane::ALL.iter().map(|p| VTab::new(p.title())).collect();
        let pane_index = Pane::ALL.iter().position(|p| *p == pane).unwrap_or(0);

        // The board is built once and sits outside every fade: switching
        // panes re-colours it, it does not replace it.
        let board = (section == Section::Keyboard).then(|| self.render_board(cx));

        let panel = match section {
            Section::Keyboard => match pane {
                Pane::Actuation => self.render_actuation(cx),
                Pane::Keys => self.render_keys(cx),
                Pane::Lighting => self.render_lighting(cx),
            },
            Section::Settings => self.render_settings(cx),
        };

        // `arriving` is the toolkit's own fade-and-settle. Keyed on section
        // and pane together, so a change of either brings the panel in and
        // neither touches the board.
        let panel = arriving(
            "panel",
            (section as u64) << 8 | pane as u64,
            &self.controls,
            panel,
        );

        let title = self
            .devices
            .first()
            .map(|d| d.display_name())
            .unwrap_or_else(|| "No keyboard".into());
        let scope = if self.selection.is_empty() {
            "Every key".to_string()
        } else {
            format!("{} keys selected", self.selection.len())
        };
        let status = self.status.clone();
        let error = self.error.clone();
        let live = self.monitoring || self.calibrating;
        let live_id: ElementId = "live-indicator".into();
        let live_t = self.controls.blend(&live_id, live, SWITCH_SLIDE);
        let live_label = if self.calibrating {
            "Calibrating"
        } else {
            "Showing live key travel"
        };

        // Built here so the borrow of `self` ends before the tree is.
        let menu_items = self.view_menu();

        // Asked once per render and only after every control has been built:
        // it counts the frame and retires records nothing drew this time. A
        // slide started during render is invisible to a check made before it.
        if self.controls.animating() {
            window.request_animation_frame();
        }

        vampir::root(div().id("root"), self, cx)
            .flex()
            .flex_col()
            .size_full()
            .bg(lit(palette.area_surface, 0.03))
            .text_color(palette.text_primary)
            .font_family(ui_font())
            .text_size(px(TEXT_SIZE))
            // The toolkit ends its own drags; this is the application's own
            // business, applying what a finished drag changed.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _w, cx| this.apply_pending(cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(16.0))
                    .px(px(20.0))
                    .pt(px(14.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .text_size(px(TITLE_TEXT_SIZE))
                                    .text_color(palette.text_primary)
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(note(palette, status)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .children((live_t > 0.001).then(|| {
                                div()
                                    .text_size(px(SMALL_TEXT_SIZE))
                                    .text_color(palette.text_secondary)
                                    .opacity(live_t)
                                    .child(live_label)
                            }))
                            .child(menu_button("view-menu", "View", true, palette, self, cx)),
                    ),
            )
            .child(
                div()
                    .id("body")
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(BODY_PAD))
                    .py(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(16.0))
                    .children(board)
                    .children((section == Section::Keyboard).then(|| {
                        tab_bar(
                            "panes",
                            &pane_tabs,
                            pane_index,
                            palette,
                            self,
                            cx,
                            |this, index, _window, cx| {
                                if let Some(next) = Pane::ALL.get(index) {
                                    this.pane = *next;
                                }
                                cx.notify();
                            },
                            |_this, _index, _window, _cx| {},
                        )
                    }))
                    .child(panel),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(20.0))
                    .py(px(10.0))
                    .border_t_1()
                    .border_color(palette.area_border)
                    .child(note(palette, scope))
                    .children(error.map(|e| {
                        div()
                            .text_size(px(SMALL_TEXT_SIZE))
                            .text_color(palette.danger_label)
                            .child(e)
                            .into_any_element()
                    })),
            )
            // Rendered once at the root so it floats above every pane.
            .children(context_menu(
                "view-menu",
                &menu_items,
                palette,
                self,
                cx,
                |this, action, window, cx| {
                    this.controls.close_menu();
                    match action {
                        "keyboard" => this.show_section(Section::Keyboard, window, cx),
                        "settings" => this.show_section(Section::Settings, window, cx),
                        "reload" => {
                            if this.require_device() {
                                this.worker.send(Cmd::Reload);
                            }
                        }
                        "release" => this.worker.send(Cmd::Disconnect),
                        _ => {}
                    }
                    cx.notify();
                },
            ))
    }
}

/// A menu row that greys out rather than disappearing when there is no
/// keyboard, so the menu keeps its shape.
fn device_item(id: &'static str, label: &'static str, connected: bool) -> MenuItem {
    let item = MenuItem::action(id, label);
    if connected { item } else { item.disabled() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SPAWN declares no `lightingConfig`, so the vendor's extra effects
    /// are never added for it. Offering one leaves the keyboard dark, which
    /// is exactly the bug this guards against.
    #[test]
    fn only_effects_this_board_supports_are_offered() {
        const INVALID: [u8; 4] = [23, 24, 25, 254];
        for m in LED_MODES {
            assert!(
                !INVALID.contains(&m.value),
                "effect {} ({}) is not valid for this board",
                m.value,
                m.name
            );
            assert!(
                (1..=19).contains(&m.value) || m.value == CUSTOM_LIGHTING,
                "effect {} is outside the supported range",
                m.value
            );
        }
    }

    #[test]
    fn per_key_mode_is_offered_and_correct() {
        assert_eq!(CUSTOM_LIGHTING, 128);
        assert!(LED_MODES.iter().any(|m| m.value == CUSTOM_LIGHTING));
    }

    #[test]
    fn effects_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for m in LED_MODES {
            assert!(seen.insert(m.value), "duplicate effect {}", m.value);
        }
    }

    /// Verified on hardware: with colour mode 1 the firmware picks its own
    /// colours and ignores the RGB, so choosing a colour must also set 0.
    #[test]
    fn choosing_a_colour_requires_custom_colour_mode() {
        assert_eq!(COLOR_CUSTOM, 0);
        assert_eq!(COLOR_AUTO, 1);
    }

    #[test]
    fn lights_off_is_a_state_not_an_effect() {
        assert_eq!(LIGHTS_OFF, 0);
        assert!(!LED_MODES.iter().any(|m| m.value == LIGHTS_OFF));
    }

    /// Polling rates are firmware codes, not indices into a list. A
    /// factory-reset board reports 6.
    #[test]
    fn report_rate_codes_match_the_firmware() {
        assert_eq!(REPORT_RATES[0].0, 3, "1000 Hz is code 3");
        assert_eq!(REPORT_RATES[1].0, 5, "4000 Hz is code 5");
        assert_eq!(REPORT_RATES[2].0, 6, "8000 Hz is code 6");
        assert!(
            REPORT_RATES.iter().any(|(code, _)| *code == 6),
            "the factory default must be selectable"
        );
    }

    #[test]
    fn travel_steps_match_firmware_precision() {
        assert!((round_step(1.234) - 1.23).abs() < 1e-6);
        assert!((round_step(0.005) - 0.01).abs() < 1e-6);
    }

    /// Slider positions map back to the value the firmware expects.
    #[test]
    fn slider_ratio_round_trips_through_the_value_range() {
        for t in [0.0f32, 0.25, 0.5, 1.0] {
            let mm = value_at(MIN_TRIGGER, MAX_TRIGGER, t);
            assert!((ratio(mm, MIN_TRIGGER, MAX_TRIGGER) - t).abs() < 1e-5);
        }
        assert_eq!(ratio(-5.0, 0.0, 1.0), 0.0, "clamped low");
        assert_eq!(ratio(9.0, 0.0, 1.0), 1.0, "clamped high");
        assert_eq!(
            ratio(1.0, 2.0, 2.0),
            0.0,
            "degenerate range does not divide by zero"
        );
    }

    /// The panel transition is keyed on section and pane together, so any
    /// change of either brings the panel in and no two states collide.
    #[test]
    fn every_section_and_pane_has_its_own_transition_key() {
        let mut seen = std::collections::HashSet::new();
        for section in Section::ALL {
            for pane in Pane::ALL {
                let key = (section as u64) << 8 | pane as u64;
                assert!(seen.insert(key), "duplicate panel key");
            }
        }
        assert_eq!(seen.len(), Section::ALL.len() * Pane::ALL.len());
    }

    /// A tab bar keys its records by label, so repeats would share them.
    #[test]
    fn tab_labels_are_unique() {
        let mut panes = std::collections::HashSet::new();
        for pane in Pane::ALL {
            assert!(panes.insert(pane.title()), "duplicate pane title");
        }
    }

    /// Two elements sharing an animation id share one animation, so a
    /// selected swatch would drag an unrelated one along with it.
    #[test]
    fn swatch_colours_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        for (_, c) in SWATCHES {
            assert!(seen.insert((c.r, c.g, c.b)), "duplicate swatch colour");
        }
    }

    /// Brightness and speed are 1..=5. A wider slider looks like more range
    /// and is not: the firmware misreads anything above the top.
    #[test]
    fn lighting_levels_stay_inside_the_range_the_firmware_honours() {
        assert_eq!(MIN_LEVEL, 1.0);
        assert_eq!(MAX_LEVEL, 5.0);
        for t in [0.0f32, 0.5, 1.0] {
            let v = value_at(MIN_LEVEL, MAX_LEVEL, t).round() as u8;
            assert!((1..=5).contains(&v), "slider produced {v}");
        }
    }

    /// Checked against the vendor's own character table: buttons travel on
    /// function byte 1, the wheel on 3, and down is -1 rather than a count.
    #[test]
    fn mouse_actions_match_the_vendor_table() {
        let by_name = |name: &str| {
            ASSIGNABLE
                .iter()
                .flat_map(|(_, list)| list.iter())
                .find(|(n, _)| *n == name)
                .map(|(_, a)| *a)
                .expect("assignment is offered")
        };
        assert_eq!(by_name("Left click"), KeyAction::mouse(1, 1));
        assert_eq!(by_name("Right click"), KeyAction::mouse(1, 2));
        assert_eq!(by_name("Middle click"), KeyAction::mouse(1, 4));
        assert_eq!(by_name("Back"), KeyAction::mouse(1, 8));
        assert_eq!(by_name("Forward"), KeyAction::mouse(1, 16));
        assert_eq!(by_name("Scroll up"), KeyAction::mouse(3, 1));
        assert_eq!(by_name("Scroll down"), KeyAction::mouse(3, 255));
    }

    /// Consumer usages, likewise from the vendor's table rather than from a
    /// generic HID list that happens to agree.
    #[test]
    fn media_actions_match_the_vendor_table() {
        let media = media_actions();
        let usage = |name: &str| {
            media
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, a)| u16::from(a.p1) | (u16::from(a.p2) << 8))
                .expect("action is offered")
        };
        assert_eq!(usage("Play or pause"), 205);
        assert_eq!(usage("Stop"), 183);
        assert_eq!(usage("Previous track"), 182);
        assert_eq!(usage("Next track"), 181);
        assert_eq!(usage("Volume up"), 233);
        assert_eq!(usage("Volume down"), 234);
        assert_eq!(usage("Mute"), 226);
    }

    /// These drive LEDs, so every swatch sits on channel extremes. A tinted
    /// "green" like `#3CE66E` lights up as teal, which is what sent me
    /// looking for a channel-order bug that was never there.
    #[test]
    fn swatches_are_the_colours_they_claim_to_be() {
        let by_name = |name: &str| {
            SWATCHES
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, c)| *c)
                .expect("swatch is offered")
        };
        assert_eq!(by_name("Red"), DevRgb::new(255, 0, 0));
        assert_eq!(by_name("Green"), DevRgb::new(0, 255, 0));
        assert_eq!(by_name("Blue"), DevRgb::new(0, 0, 255));
        assert_eq!(by_name("Cyan"), DevRgb::new(0, 255, 255));
        assert_eq!(by_name("Magenta"), DevRgb::new(255, 0, 255));

        // Green and cyan must not be near neighbours: telling them apart is
        // the whole point of offering both.
        let g = by_name("Green");
        let c = by_name("Cyan");
        assert_ne!(g, c);
        assert_eq!(g.b, 0, "green carries no blue at all");

        // Every channel is either off or full, bar the two deliberate mixes.
        for (name, c) in SWATCHES {
            if matches!(name, "Orange" | "Purple") {
                continue;
            }
            for (channel, v) in [("r", c.r), ("g", c.g), ("b", c.b)] {
                assert!(v == 0 || v == 255, "{name} has a partial {channel} channel");
            }
        }
    }

    /// The window has to open wide enough for the board, which is a fixed
    /// width whatever the panel under it is showing.
    #[test]
    fn the_board_fits_the_default_window() {
        // Every row is the same width, however it is divided into caps.
        for row in SPAWN_ROWS {
            let width: f32 = row
                .iter()
                .map(|k| UNIT * k.width + GAP * (k.width - 1.0))
                .sum::<f32>()
                + GAP * (row.len() - 1) as f32;
            assert!(
                (width - (UNIT * BOARD_UNITS + GAP * (BOARD_UNITS - 1.0))).abs() < 0.5,
                "a row came out {width}px"
            );
        }
        assert!(board_width() > 800.0, "the board is wider than that");
        assert!(
            crate::WINDOW_WIDTH >= board_width(),
            "the default window clips the board"
        );
        assert!(
            crate::WINDOW_MIN_WIDTH >= board_width(),
            "the window can be dragged narrower than the board"
        );
    }

    /// The lighting levels must actually be routed through `stepped_level`.
    /// Testing the algorithm proves nothing if the app never calls it, which
    /// is exactly how this fault survived a passing test once already.
    #[test]
    fn lighting_levels_are_routed_through_the_stepper() {
        let source = include_str!("app.rs");
        let needle = concat!("stepped_", "level(dragging, at.x, self.led.");
        assert_eq!(
            source.matches(needle).count(),
            2,
            "brightness and speed both step through the same place"
        );
        // And nothing reads the track position straight into a level.
        assert!(
            !source.contains(concat!("self.led.brightness = value_", "at(")),
            "brightness bypasses the stepper"
        );
        assert!(
            !source.contains(concat!("self.led.speed = value_", "at(")),
            "speed bypasses the stepper"
        );
    }

    /// Vampir 0.1.2's arrow-key arithmetic, so the test drives the real
    /// function with the real input rather than a copy of either.
    fn vampir_nudge(ratio_now: f32, forward: bool) -> f32 {
        const STOPS: i64 = 5;
        let current = (ratio_now * STOPS as f32).round() as i64;
        let moved = if forward { current + 1 } else { current - 1 };
        moved.clamp(0, STOPS) as f32 / STOPS as f32
    }

    /// Stepping by direction reaches every level both ways, which taking the
    /// ratio at face value does not.
    #[test]
    fn keyboard_stepping_reaches_every_level() {
        let mut level = 5u8;
        for expected in [4u8, 3, 2, 1, 1] {
            let at = vampir_nudge(ratio(f32::from(level), MIN_LEVEL, MAX_LEVEL), false);
            level = stepped_level(false, at, level);
            assert_eq!(level, expected, "stepping down");
        }

        let mut level = 1u8;
        for expected in [2u8, 3, 4, 5, 5] {
            let at = vampir_nudge(ratio(f32::from(level), MIN_LEVEL, MAX_LEVEL), true);
            level = stepped_level(false, at, level);
            assert_eq!(level, expected, "stepping up");
        }
    }

    /// Three is where it used to stall, so it gets its own check.
    #[test]
    fn the_left_arrow_leaves_three() {
        let at = vampir_nudge(ratio(3.0, MIN_LEVEL, MAX_LEVEL), false);
        assert_eq!(stepped_level(false, at, 3), 2, "arrow ratio was {at}");
    }

    /// A drag still reads the track directly: the mouse and the ticks agree
    /// with each other, and only the keyboard needed working around.
    #[test]
    fn dragging_reads_the_track_position() {
        assert_eq!(stepped_level(true, 0.0, 3), 1);
        assert_eq!(stepped_level(true, 0.5, 3), 3);
        assert_eq!(stepped_level(true, 1.0, 3), 5);
        // Out-of-range positions cannot produce an out-of-range level.
        assert_eq!(stepped_level(true, -1.0, 3), 1);
        assert_eq!(stepped_level(true, 2.0, 3), 5);
    }

    /// Home and End reach the ends in one press.
    #[test]
    fn the_ends_are_one_press_away() {
        assert_eq!(stepped_level(false, 0.0, 4), 1);
        assert_eq!(stepped_level(false, 1.0, 2), 5);
    }

    /// Reproduces the stepped-slider keyboard fault, upstream in Vampir
    /// 0.1.2.
    ///
    /// `tick_marks` draws `stops` ticks at `i / (stops - 1)`, and
    /// `track_ratio_at` snaps a drag the same way, so `stops` is a count of
    /// *positions*. `nudge_stepped` instead computes `(ratio * stops).round()`
    /// and divides by `stops`, treating it as a count of *intervals* — one
    /// position too many, landing between the ticks the mouse snaps to.
    ///
    /// For brightness, which runs 1..=5 over five positions, that leaves the
    /// value stuck: at 3 the left arrow produces a ratio that rounds straight
    /// back to 3.
    #[test]
    fn stepped_sliders_stall_on_the_keyboard_upstream() {
        const STOPS: u32 = 5;
        // Vampir 0.1.2's arrow-key arithmetic, copied exactly.
        let vampir_nudge_left = |ratio: f32| {
            let current = (ratio * STOPS as f32).round() as i64;
            (current - 1).clamp(0, STOPS as i64) as f32 / STOPS as f32
        };
        // How the mouse and the tick marks divide the same track.
        let mouse_snap = |ratio: f32| {
            let last = (STOPS - 1) as f32;
            (ratio * last).round() / last
        };

        let level = |ratio: f32| value_at(MIN_LEVEL, MAX_LEVEL, ratio).round() as u8;
        let ratio_of = |level: u8| ratio(f32::from(level), MIN_LEVEL, MAX_LEVEL);

        // Stepping down from 5 works until it reaches 3, and then stops.
        assert_eq!(level(vampir_nudge_left(ratio_of(5))), 4);
        assert_eq!(level(vampir_nudge_left(ratio_of(4))), 3);
        assert_eq!(
            level(vampir_nudge_left(ratio_of(3))),
            3,
            "this is the fault: the left arrow at 3 yields 3 again"
        );

        // The same step done the way the mouse does it keeps going.
        let below_three = mouse_snap(ratio_of(3) - 1.0 / (STOPS - 1) as f32);
        assert_eq!(level(below_three), 2, "the mouse can reach 2");
    }

    /// The toolkit owns the keyboard, the mouse, the scheme and the type
    /// scale as of 0.1.2. Keeping a second copy of any of them is how the
    /// two drift apart.
    #[test]
    fn the_toolkit_owns_what_it_provides() {
        let source = include_str!("app.rs");
        for (own, what) in [
            (concat!("fn system_", "dark"), "vampir::system_dark"),
            (concat!("actions!(", "spawn"), "vampir::Dismiss"),
            (concat!("fn root_mouse_", "move"), "vampir::handle_mouse"),
            (concat!("px(12", ".5)"), "vampir::TEXT_SIZE"),
            (concat!("px(11", ".5)"), "vampir::SMALL_TEXT_SIZE"),
        ] {
            assert!(
                !source.contains(own),
                "this re-implements what {what} provides"
            );
        }
        // And the root goes through the toolkit, which wires focus, Escape,
        // Tab and the drag pump in one call.
        assert!(source.contains(concat!("vampir::", "root(")));
    }

    /// The picker works in OKLCH; the keyboard takes sRGB bytes. Round-trip
    /// the ends of the range so a mix cannot silently land somewhere else.
    #[test]
    fn the_picker_converts_to_device_bytes() {
        let black = oklch_to_device(Oklch::new(0.0, 0.0, 0.0));
        assert_eq!(black, DevRgb::new(0, 0, 0));

        let white = oklch_to_device(Oklch::new(0.0, 0.0, 1.0));
        assert_eq!(white, DevRgb::new(255, 255, 255));

        // Somewhere in the greens stays in the greens.
        let green = oklch_to_device(Oklch::new(150.0, MAX_CHROMA, 0.72));
        assert!(green.g > green.r && green.g > green.b, "got {green:?}");

        // Every hue lands inside the byte range rather than wrapping.
        for hue in (0..360).step_by(15) {
            let c = oklch_to_device(Oklch::new(f64::from(hue), MAX_CHROMA, 0.7));
            let _ = (c.r, c.g, c.b);
        }
    }

    /// Both routes to a colour go through one place, so per-key painting and
    /// the effect record cannot drift apart.
    #[test]
    fn picking_a_colour_has_a_single_path() {
        let source = include_str!("app.rs");
        let needle = concat!("fn choose_", "color");
        assert_eq!(source.matches(needle).count(), 1);
    }

    /// The destructive actions start folded away on every launch. A
    /// disclosure that reopens itself defeats the point of hiding them.
    #[test]
    fn the_danger_zone_starts_closed() {
        let source = include_str!("app.rs");
        assert!(
            source.contains("danger_open: false"),
            "the danger zone must be closed at construction"
        );
        // It is also the only place the resets live. Split so this test's own
        // copy of the string is not one of the hits.
        let needle = concat!("ResetScope", "::All");
        assert_eq!(
            source.matches(needle).count(),
            1,
            "a factory reset is reachable from one place only"
        );
    }

    /// Direction values are the firmware's, not list positions. The vendor
    /// sends 2/3 for a vertical effect and 1/0 for a horizontal one, so
    /// numbering them by index would have got every one of them wrong.
    #[test]
    fn direction_values_match_the_vendor_arrows() {
        assert_eq!((DIR_RIGHT, DIR_LEFT, DIR_UP, DIR_DOWN), (0, 1, 2, 3));

        let vertical = directions_for(10).expect("effect 10 travels vertically");
        assert_eq!(vertical, [(DIR_UP, "Up"), (DIR_DOWN, "Down")]);

        for mode in [11, 12, 16, 18] {
            let pair = directions_for(mode).expect("effect travels horizontally");
            assert_eq!(
                pair,
                [(DIR_LEFT, "Left"), (DIR_RIGHT, "Right")],
                "effect {mode}"
            );
        }

        assert!(
            directions_for(1).is_none(),
            "a static effect has no direction"
        );
        assert!(
            directions_for(23).is_none(),
            "effects this board does not have must not offer one either"
        );
    }

    /// Anything with a direction has to be an effect the board actually has.
    #[test]
    fn directional_effects_are_offered() {
        for mode in [10, 11, 12, 16, 18] {
            assert!(
                LED_MODES.iter().any(|m| m.value == mode),
                "effect {mode} has directions but is not in the list"
            );
        }
    }

    /// The profile list must name switches, not translation keys. `axis_6`
    /// and friends are i18n identifiers; showing them means the setting
    /// cannot be answered by looking at the keyboard.
    #[test]
    fn switch_profiles_are_named_not_keyed() {
        for (value, name, _) in AXIS_PROFILES {
            assert!(
                !name.starts_with("Axis ") && !name.contains('_'),
                "profile {value} is labelled with its key, not its name: {name}"
            );
        }
        // Values are the firmware's, and the vendor's default is 1.
        let values: Vec<u8> = AXIS_PROFILES.iter().map(|(v, _, _)| *v).collect();
        assert_eq!(
            values,
            vec![1, 4, 5, 6, 2, 3],
            "order and values match the vendor list"
        );
        assert_eq!(RtKey::default().axis_type, 1, "the vendor default switch");
    }

    /// The vendor bounds dead zones at half a millimetre, and takes the
    /// rapid-trigger ceiling from the selected switch.
    #[test]
    fn ranges_match_the_vendor_bounds() {
        assert_eq!(MAX_DEAD_ZONE, 0.5);
        assert_eq!(MIN_TRIGGER, 0.10);
        assert_eq!(MAX_TRIGGER, 3.40);
        assert_eq!(MIN_RT, 0.01);
        assert_eq!(max_rt_for(1), 3.4, "axis 6");
        assert_eq!(max_rt_for(2), 3.3, "jade");
        assert_eq!(max_rt_for(3), 3.3, "king");
        assert_eq!(max_rt_for(6), 3.4, "axis 20");
        assert_eq!(max_rt_for(200), MAX_RT, "an unknown switch falls back");
    }

    /// The board is the only keyboard on screen; a pane that draws its own
    /// would stack a second one under it.
    #[test]
    fn only_one_board_is_rendered() {
        let source = include_str!("app.rs");
        // Split so this test's own copy of the string is not one of the hits.
        let needle = concat!("self.render_", "board(cx)");
        assert_eq!(
            source.matches(needle).count(),
            1,
            "the board is hoisted out of the panes and drawn once"
        );
    }

    /// A remapped key shows what it does now; an untouched one keeps its
    /// legend.
    #[test]
    fn only_remapped_keys_get_a_new_label() {
        assert_eq!(assigned_label(&KeyAction::default()), None);
        assert_eq!(
            assigned_label(&KeyAction::keyboard(0, 0xE0)),
            Some("Left Ctrl")
        );
        assert_eq!(
            assigned_label(&KeyAction::consumer(0x00E9)),
            Some("Volume up")
        );
    }

    /// Every slider id must be routed in `track_dragged`, or dragging it
    /// silently does nothing.
    #[test]
    fn every_slider_id_is_routed() {
        const IDS: [&str; 9] = [
            "led-hue",
            "led-pad",
            "trigger",
            "press-rt",
            "release-rt",
            "brightness",
            "speed",
            "top-dead-zone",
            "bottom-dead-zone",
        ];
        let source = include_str!("app.rs");
        for id in IDS {
            let arm = format!("\"{id}\" =>");
            assert!(
                source.contains(&arm),
                "slider `{id}` has no track_dragged arm"
            );
        }
    }
}
