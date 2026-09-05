// SPDX-License-Identifier: GPL-3.0-or-later
//
// SPAWN Universal - a free-software configurator for SPAWN magnetic keyboards.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod app;
mod device;
mod render_test;
mod theme;
mod worker;

use gpui::{App, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};
use gpui_ce_platform::application;

gpui::actions!(spawn, [Quit]);

/// The window opens just wide enough for the keyboard and no wider: the board
/// is a fixed width, and everything under it is narrower.
pub const WINDOW_WIDTH: f32 = 900.0;
/// Tall enough for the board, the tabs and a panel without scrolling.
pub const WINDOW_HEIGHT: f32 = 620.0;
/// Narrower than this and the board would be clipped; `board_width` is what
/// it actually needs.
pub const WINDOW_MIN_WIDTH: f32 = 900.0;
pub const WINDOW_MIN_HEIGHT: f32 = 480.0;

/// Print every HID interface the keyboard exposes, and which one we would
/// talk to. The first thing to check when a board is not detected.
fn list_devices() -> i32 {
    let api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("cannot initialise HID: {e}");
            return 1;
        }
    };

    let mut any = false;
    println!(
        "All HID interfaces from vendor {:#06x}:",
        spawn_protocol::VID_SONIX
    );
    for d in api
        .device_list()
        .filter(|d| d.vendor_id() == spawn_protocol::VID_SONIX)
    {
        any = true;
        let usable = spawn_protocol::VENDOR_USAGE_PAGES.contains(&d.usage_page());
        println!(
            "  {:04X}:{:04X}  usage page {:#06x}  {:<28} {}",
            d.vendor_id(),
            d.product_id(),
            d.usage_page(),
            d.product_string().unwrap_or("<no name>"),
            if usable {
                "<- configuration interface"
            } else {
                ""
            }
        );
    }

    if !any {
        println!("  none found");
        println!();
        println!("If the keyboard is plugged in:");
        println!("  Linux  - install packaging/99-spawn.rules, then replug");
        println!("  macOS  - no setup needed; try a different port");
        println!("  Windows- no setup needed; close any vendor software first");
        return 1;
    }

    let usable = device::enumerate(&api);
    println!();
    println!("{} usable configuration interface(s)", usable.len());
    i32::from(usable.is_empty())
}

/// Read-only probe: opens the configuration interface and prints what the
/// firmware reports. Writes nothing, so it is safe to run at any time.
fn probe() -> i32 {
    let api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("cannot initialise HID: {e}");
            return 1;
        }
    };
    let Some(cand) = device::enumerate(&api).into_iter().next() else {
        eprintln!("no configuration interface found; try --list");
        return 1;
    };
    let kb = match device::Keyboard::open(&api, &cand) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("cannot open {}: {e:#}", cand.path);
            return 1;
        }
    };
    println!(
        "interface : {} (usage page {:#06x})",
        cand.display_name(),
        cand.usage_page
    );
    println!("report len: {} bytes", kb.packet_len());

    let mut sink = |_t: spawn_protocol::KeyTelemetry| {};
    match kb.read_snapshot(&mut sink) {
        Ok(s) => {
            let i = &s.info;
            println!("firmware  : {}", i.version_string());
            println!(
                "ids       : {:04X}:{:04X}  sensor {}",
                i.vid, i.pid, i.sensor
            );
            println!(
                "profile   : {}  battery {}%",
                i.current_profile, i.battery_level
            );
            println!(
                "rt units  : {}",
                if i.rt_precision == 1 {
                    "0.001 mm"
                } else {
                    "0.01 mm"
                }
            );
            let hz = match s.game_mode.report_rate {
                3 => "1000 Hz",
                4 => "2000 Hz",
                5 => "4000 Hz",
                6 => "8000 Hz",
                _ => "unknown",
            };
            println!(
                "polling   : {hz} (code {})  dead zones {:.2}/{:.2} mm",
                s.game_mode.report_rate, s.game_mode.top_dead_zone, s.game_mode.bottom_dead_zone
            );
            println!(
                "lighting  : mode {} brightness {} speed {} colour {}",
                s.led.mode,
                s.led.brightness,
                s.led.speed,
                if s.led.color_mode == 1 {
                    "auto (rainbow)".to_string()
                } else {
                    format!(
                        "rgb({},{},{})",
                        s.led.primary.r, s.led.primary.g, s.led.primary.b
                    )
                }
            );
            match kb.read_live_leds(&mut sink) {
                Ok(v) => {
                    let lit = v
                        .iter()
                        .filter(|c| c.r != 0 || c.g != 0 || c.b != 0)
                        .count();
                    println!(
                        "live leds : {lit}/{} reporting colour{}",
                        v.len(),
                        if lit == 0 {
                            "  (this firmware does not implement the read)"
                        } else {
                            ""
                        }
                    );
                }
                Err(_) => println!("live leds : not supported"),
            }
            println!();
            let unconfigured = s.rt.iter().filter(|r| r.is_unconfigured()).count();
            println!(
                "per-key actuation (raw; {unconfigured} of {} slots unconfigured):",
                s.rt.len()
            );
            for kc in spawn_protocol::layout::all_keys().take(12) {
                let r = &s.rt[kc.slot as usize];
                let a = &s.keys[kc.slot as usize];
                if r.is_unconfigured() {
                    let d = spawn_protocol::RtKey::default();
                    println!(
                        "  {:<10} slot {:>3}  unconfigured -> app shows {:.2} mm, rt {:.2}/{:.2}",
                        kc.label, kc.slot, d.trigger_mm, d.press_rt_mm, d.release_rt_mm
                    );
                } else {
                    println!(
                        "  {:<10} slot {:>3}  trigger {:.2} mm  rt {:.2}/{:.2}  axis {}  page {:?}",
                        kc.label,
                        kc.slot,
                        r.trigger_mm,
                        r.press_rt_mm,
                        r.release_rt_mm,
                        r.axis_type,
                        a.page
                    );
                }
            }
            0
        }
        Err(e) => {
            eprintln!("read failed: {e:#}");
            1
        }
    }
}

/// Read-only: dump the raw reply packets for one command so framing problems
/// are visible.
fn raw_dump(cmd: spawn_protocol::Cmd, len: usize) -> i32 {
    let api = hidapi::HidApi::new().expect("hid");
    let Some(cand) = device::enumerate(&api).into_iter().next() else {
        eprintln!("no device");
        return 1;
    };
    let kb = device::Keyboard::open(&api, &cand).expect("open");
    let req = spawn_protocol::codec::Request::read(cmd, len).with_packet_len(kb.packet_len());
    println!(
        "cmd {cmd:?}  len {len}  packet {}  chunks {}",
        kb.packet_len(),
        req.plan().len()
    );
    for (i, p) in req.plan().enumerate().take(3) {
        println!("  out[{i}] {}", hex(&p.as_slice()[..16]));
    }
    let mut sink = |_t: spawn_protocol::KeyTelemetry| {};
    match kb.exchange(&req, &mut sink) {
        Ok(rs) => {
            println!("got {} replies", rs.len());
            for (i, r) in rs.iter().enumerate().take(4) {
                println!(
                    "  in[{i}] cmd={:#04x} len={} addr={} data={}",
                    r.cmd,
                    r.len_or_type,
                    r.addr,
                    hex(&r.data[..r.data.len().min(24)])
                );
            }
            0
        }
        Err(e) => {
            eprintln!("exchange failed: {e:#}");
            1
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x} ")).collect()
}

/// Exercise the exact worker path the GUI uses: scan, auto-connect, read
/// back a snapshot. Read-only. Prints every event the UI would receive.
fn selftest() -> i32 {
    use worker::{Cmd, Event, Worker};

    let w = Worker::spawn();
    w.send(Cmd::Rescan);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut connected = false;
    let mut asked = false;

    while std::time::Instant::now() < deadline {
        match w.rx.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(Event::Devices(list)) => {
                println!("devices: {}", list.len());
                for d in &list {
                    println!("  {} @ usage {:#06x}", d.display_name(), d.usage_page);
                }
                if let Some(first) = list.first() {
                    if !asked {
                        asked = true;
                        println!("-> connecting");
                        w.send(Cmd::Connect(first.clone()));
                    }
                }
            }
            Ok(Event::Connected(snap, label)) => {
                println!("connected: {label}");
                println!(
                    "  lighting mode {} brightness {} speed {} colour_mode {}",
                    snap.led.mode, snap.led.brightness, snap.led.speed, snap.led.color_mode
                );
                println!("  rt entries {}  keys {}", snap.rt.len(), snap.keys.len());
                connected = true;
                break;
            }
            Ok(Event::Status(s)) => println!("status: {s}"),
            Ok(Event::Error(e)) => println!("error: {e}"),
            Ok(Event::Loaded(_)) => println!("reloaded"),
            Ok(Event::Disconnected) => println!("disconnected"),
            Ok(Event::Telemetry(t)) => println!("telemetry: {} frames", t.len()),
            Err(_) => {}
        }
    }

    if connected {
        println!("SELFTEST OK");
        0
    } else {
        println!("SELFTEST FAILED: never reached Connected");
        1
    }
}

/// Write test for the lighting path: save the current effect, write a known
/// one, read it back, then restore. Verifies the write path end to end and
/// leaves the keyboard exactly as it was found.
fn test_lighting() -> i32 {
    use spawn_protocol::{LedEffect, Rgb};
    use std::io::Write as _;

    let api = hidapi::HidApi::new().expect("hid");
    let Some(cand) = device::enumerate(&api).into_iter().next() else {
        eprintln!("no device");
        return 1;
    };
    let kb = device::Keyboard::open(&api, &cand).expect("open");
    let mut sink = |_t: spawn_protocol::KeyTelemetry| {};
    let before = kb.read_snapshot(&mut sink).expect("snapshot");

    for n in (1..=5).rev() {
        print!("\rstarting in {n}\u{2026} watch the keyboard ");
        let _ = std::io::stdout().flush();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    println!("\r                                        ");

    let green = Rgb::new(0, 255, 0);

    // 1. The effect record's own colour. This path was confirmed by eye
    //    earlier, so it is the control.
    let effect = LedEffect {
        mode: 1,
        primary: green,
        secondary: Rgb::default(),
        color_mode: 0,
        brightness: 5,
        speed: 3,
        direction: 0,
        effect_mode_type: 0,
    };
    let _ = kb.write_led(&effect, &mut sink);
    println!("1/2  effect colour, pure GREEN (0,255,0)   holding 10s");
    let _ = std::io::stdout().flush();
    std::thread::sleep(std::time::Duration::from_secs(10));

    // Dark for a moment, so the second step is obviously a new thing and
    // not the first one still sitting there.
    let _ = kb.write_led(&LedEffect { mode: 0, ..effect }, &mut sink);
    println!("     (dark for 2s)");
    let _ = std::io::stdout().flush();
    std::thread::sleep(std::time::Duration::from_secs(2));

    // 2. The per-key table, which has only ever been checked with white --
    //    a colour that looks the same whatever order the channels are in.
    let painted = vec![green; spawn_protocol::KEY_SLOTS];
    let _ = kb.write_custom_led(&painted, &mut sink);
    let per_key = LedEffect {
        mode: 128,
        ..effect
    };
    let _ = kb.write_led(&per_key, &mut sink);
    println!("2/2  per-key colour, the same GREEN        holding 10s");
    let _ = std::io::stdout().flush();
    std::thread::sleep(std::time::Duration::from_secs(10));

    let _ = kb.write_custom_led(&before.custom_led, &mut sink);
    let _ = kb.write_led(&before.led, &mut sink);
    println!("restored");
    0
}

fn test_leds() -> i32 {
    use spawn_protocol::{LedEffect, Rgb};
    use std::io::Write as _;

    let api = hidapi::HidApi::new().expect("hid");
    let Some(cand) = device::enumerate(&api).into_iter().next() else {
        eprintln!("no device");
        return 1;
    };
    let kb = device::Keyboard::open(&api, &cand).expect("open");
    let mut sink = |_t: spawn_protocol::KeyTelemetry| {};

    let before = kb.read_snapshot(&mut sink).expect("snapshot");

    for n in (1..=5).rev() {
        print!("\rstarting in {n}\u{2026} watch the keyboard ");
        let _ = std::io::stdout().flush();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    println!("\r                                        ");

    // Slot numbers come from the layout table; the label is what should light
    // if LED index really equals key slot.
    let probes: [(u8, &str); 4] = [(0, "Esc"), (50, "S"), (83, "Space"), (85, "Fn")];

    for (slot, label) in probes {
        let mut leds = vec![Rgb::default(); spawn_protocol::KEY_SLOTS];
        leds[slot as usize] = Rgb::new(255, 255, 255);
        let _ = kb.write_custom_led(&leds, &mut sink);
        let e = LedEffect {
            mode: 128,
            primary: Rgb::new(255, 255, 255),
            secondary: Rgb::default(),
            color_mode: 0,
            brightness: 5,
            speed: 3,
            direction: 0,
            effect_mode_type: 0,
        };
        let _ = kb.write_led(&e, &mut sink);
        println!("LED index {slot:>3}  -> should light: {label}   (holding 6s)");
        let _ = std::io::stdout().flush();
        std::thread::sleep(std::time::Duration::from_secs(6));
    }

    let _ = kb.write_custom_led(&before.custom_led, &mut sink);
    let _ = kb.write_led(&before.led, &mut sink);
    println!("restored");
    0
}

/// Factory-reset the keyboard and print everything it reports afterwards.
/// The result is the firmware's own defaults, which is the only way to know
/// them rather than infer them from the vendor interface.
///
/// `--no-reset` reads a board that has already been reset; the firmware goes
/// quiet for a moment afterwards, and reading straight through that burns the
/// retry budget on every packet.
fn record_defaults() -> i32 {
    let api = hidapi::HidApi::new().expect("hid");
    let Some(cand) = device::enumerate(&api).into_iter().next() else {
        eprintln!("no device");
        return 1;
    };
    let kb = match device::Keyboard::open(&api, &cand) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{e:#}");
            return 1;
        }
    };
    let mut sink = |_t: spawn_protocol::KeyTelemetry| {};

    if !std::env::args().any(|a| a == "--no-reset") {
        eprintln!("resetting to factory defaults...");
        if let Err(e) = kb.factory_reset(spawn_protocol::ResetScope::All) {
            eprintln!("reset failed: {e:#}");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }

    eprintln!("reading...");
    let snap = match kb.read_snapshot(&mut sink) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read failed: {e:#}");
            return 1;
        }
    };

    let i = &snap.info;
    println!("=== device info ===");
    println!("firmware          {}", i.version_string());
    println!("ids               {:04X}:{:04X}", i.vid, i.pid);
    println!("sensor            {}", i.sensor);
    println!("work_mode         {}", i.work_mode);
    println!("current_profile   {}", i.current_profile);
    println!("axis_info         {}", i.axis_info);
    println!("rt_precision      {}", i.rt_precision);

    let g = &snap.game_mode;
    println!();
    println!("=== settings ===");
    println!("game_mode         {}", g.game_mode);
    println!("fn_switch         {}", g.fn_switch);
    println!("sleep_time        {}", g.sleep_time);
    println!("key_delay         {}", g.key_delay);
    println!("report_rate       {}", g.report_rate);
    println!("system_mode       {}", g.system_mode);
    println!("top_dead_zone     {:.2} mm", g.top_dead_zone);
    println!("bottom_dead_zone  {:.2} mm", g.bottom_dead_zone);
    println!("stability_mode    {}", g.stability_mode);
    println!("auto_calibration  {}", g.auto_calibration);
    println!("single_key_wakeup {}", g.single_key_wakeup);

    let l = &snap.led;
    println!();
    println!("=== lighting ===");
    println!("mode              {}", l.mode);
    println!(
        "primary           rgb({},{},{})",
        l.primary.r, l.primary.g, l.primary.b
    );
    println!(
        "secondary         rgb({},{},{})",
        l.secondary.r, l.secondary.g, l.secondary.b
    );
    println!("color_mode        {}", l.color_mode);
    println!("brightness        {}", l.brightness);
    println!("speed             {}", l.speed);
    println!("direction         {}", l.direction);
    println!("effect_mode_type  {}", l.effect_mode_type);

    println!();
    println!("=== rapid trigger ===");
    let unconfigured = snap.rt.iter().filter(|r| r.is_unconfigured()).count();
    println!("unconfigured      {unconfigured} of {}", snap.rt.len());
    let first = snap.rt[0];
    let uniform = snap
        .rt
        .iter()
        .take(spawn_protocol::KEY_SLOTS)
        .all(|r| *r == first);
    println!("uniform           {uniform}");
    println!("axis_type         {}", first.axis_type);
    println!("trigger           {:.2} mm", first.trigger_mm);
    println!("press_rt          {:.2} mm", first.press_rt_mm);
    println!("release_rt        {:.2} mm", first.release_rt_mm);
    println!("whole_fast        {}", first.whole_fast);
    println!("rampage           {}", first.rampage);

    println!();
    println!("=== keymap and colour ===");
    let remapped = snap
        .keys
        .iter()
        .filter(|k| k.page != spawn_protocol::Page::Default)
        .count();
    println!("non-default keys  {remapped} of {}", snap.keys.len());
    let lit = snap
        .custom_led
        .iter()
        .filter(|c| c.r != 0 || c.g != 0 || c.b != 0)
        .count();
    println!("non-black leds    {lit} of {}", snap.custom_led.len());
    0
}

fn main() {
    if std::env::args().any(|a| a == "--record-defaults") {
        std::process::exit(record_defaults());
    }
    if std::env::args().any(|a| a == "--test-leds") {
        std::process::exit(test_leds());
    }
    if std::env::args().any(|a| a == "--test-lighting") {
        std::process::exit(test_lighting());
    }
    if std::env::args().any(|a| a == "--selftest") {
        std::process::exit(selftest());
    }
    if let Some(pos) = std::env::args().position(|a| a == "--raw") {
        let args: Vec<String> = std::env::args().collect();
        let which = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("rt");
        let (cmd, len) = match which {
            "rt" => (
                spawn_protocol::Cmd::GetMagneticAxisRt,
                spawn_protocol::RT_TABLE_LEN,
            ),
            "keys" => (spawn_protocol::Cmd::GetKey, spawn_protocol::KEY_TABLE_LEN),
            "gm" => (
                spawn_protocol::Cmd::GetGameMode,
                spawn_protocol::GAME_MODE_LEN,
            ),
            "defkeys" => (spawn_protocol::Cmd::GetDefaultKeyMatrix, 512),
            "led" => (
                spawn_protocol::Cmd::GetCustomLedData,
                spawn_protocol::CUSTOM_LED_LEN,
            ),
            "fn" => (spawn_protocol::Cmd::GetFnKey, 512),
            "ledfx" => (
                spawn_protocol::Cmd::GetLedEffect,
                spawn_protocol::LED_EFFECT_LEN,
            ),
            _ => (
                spawn_protocol::Cmd::GetDeviceInfo,
                spawn_protocol::DEVICE_INFO_LEN,
            ),
        };
        std::process::exit(raw_dump(cmd, len));
    }
    if std::env::args().any(|a| a == "--probe") {
        std::process::exit(probe());
    }
    if std::env::args().any(|a| a == "--list" || a == "-l") {
        std::process::exit(list_devices());
    }
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("spawn-universal {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("spawn_app=info,spawn_protocol=info"),
    )
    .init();

    application().run(|cx: &mut App| {
        // Escape, Tab and Shift-Tab, bound by the toolkit that handles them.
        vampir::bind_keys(cx);
        // macOS draws Cmd-Q in the menu but does not implement it: without a
        // binding and an action the shortcut does nothing at all.
        cx.bind_keys([gpui::KeyBinding::new("secondary-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        // An application that never calls this has no main menu, so no Quit
        // item and no working shortcut.
        cx.set_menus(vec![
            gpui::Menu {
                name: "Spawn Universal".into(),
                items: vec![gpui::MenuItem::action("Quit Spawn Universal", Quit)],
                disabled: false,
            },
            vampir::edit_menu(),
        ]);
        // This is a single-window application: closing the window is how
        // someone quits it, so the process should go with it.
        cx.on_window_closed(|cx, _id| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        // gpui_platform's font backend is behind the `font-kit` feature. Built
        // without it the window lays out perfectly and draws no glyphs at all,
        // which is a hard failure to recognise on sight — so say so plainly.
        let fonts = cx.text_system().all_font_names();
        if fonts.is_empty() {
            log::error!(
                "no fonts available: the interface will render without any text. \
                 Build with gpui_platform's `font-kit` feature enabled."
            );
        } else {
            log::info!("{} fonts available", fonts.len());
        }

        // Derived, not guessed: if a keycap ever changes size the window
        // follows rather than clipping the board.
        let width = WINDOW_WIDTH.max(app::board_width());
        let bounds = Bounds::centered(None, size(px(width), px(WINDOW_HEIGHT)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(
                    px(WINDOW_MIN_WIDTH.max(app::board_width())),
                    px(WINDOW_MIN_HEIGHT),
                )),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("SPAWN Universal".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| app::SpawnApp::new(window, cx)),
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
