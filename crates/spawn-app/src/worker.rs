// SPDX-License-Identifier: GPL-3.0-or-later
//
// Device worker.
//
// hidapi is blocking, so the device lives on its own thread. The UI talks to
// it over channels and never blocks on I/O.

use crate::device::{Candidate, Keyboard, Snapshot, enumerate};
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded};
use hidapi::HidApi;
use spawn_protocol::{GameMode, KeyAction, KeyTelemetry, LedEffect, ResetScope, Rgb, RtKey};
use std::time::{Duration, Instant};

/// Requests from the UI to the device thread.
#[derive(Debug, Clone)]
pub enum Cmd {
    Rescan,
    Connect(Candidate),
    Disconnect,
    Reload,
    ApplyRt(Vec<RtKey>),
    ApplyLed(LedEffect),
    ApplyCustomLed(Vec<Rgb>),
    ApplyGameMode(GameMode),
    ApplyKeys(Vec<KeyAction>),
    SetMonitor(bool),
    SetCalibration(bool),
    FactoryReset(ResetScope),
    Shutdown,
}

/// Notifications from the device thread to the UI.
#[derive(Debug, Clone)]
pub enum Event {
    Devices(Vec<Candidate>),
    Connected(Box<Snapshot>, String),
    Disconnected,
    Loaded(Box<Snapshot>),
    Telemetry(Vec<KeyTelemetry>),
    Status(String),
    Error(String),
}

pub struct Worker {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Event>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    /// A worker with no device thread behind it.
    ///
    /// Tests build the real view, and the view owns a worker; without this
    /// every render test would open the developer's actual keyboard and fight
    /// the running application for exclusive access.
    #[cfg(test)]
    pub fn offline() -> Self {
        let (cmd_tx, _cmd_rx) = bounded::<Cmd>(1);
        let (_evt_tx, evt_rx) = bounded::<Event>(1);
        Self {
            tx: cmd_tx,
            rx: evt_rx,
            handle: None,
        }
    }

    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = bounded::<Cmd>(64);
        let (evt_tx, evt_rx) = bounded::<Event>(256);

        let handle = std::thread::Builder::new()
            .name("spawn-hid".into())
            .spawn(move || run(cmd_rx, evt_tx))
            .expect("failed to spawn HID thread");

        Self {
            tx: cmd_tx,
            rx: evt_rx,
            handle: Some(handle),
        }
    }

    pub fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_err() {
            log::debug!("no HID thread listening; command dropped");
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// How often to re-enumerate while nothing is connected.
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);
/// Idle sleep so the thread does not spin.
const IDLE_TICK: Duration = Duration::from_millis(8);

fn run(rx: Receiver<Cmd>, tx: Sender<Event>) {
    let mut api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            let _ = tx.send(Event::Error(format!("cannot initialise HID: {e}")));
            return;
        }
    };

    let mut kb: Option<Keyboard> = None;
    let mut monitoring = false;
    let mut last_scan = Instant::now() - RESCAN_INTERVAL;
    let mut known: Vec<Candidate> = Vec::new();

    loop {
        // ---- drain commands
        loop {
            match rx.try_recv() {
                Ok(Cmd::Shutdown) => {
                    if let Some(k) = &kb {
                        let _ = k.stop_monitor();
                    }
                    return;
                }
                Ok(cmd) => {
                    if let Err(e) = handle(cmd, &mut api, &mut kb, &mut monitoring, &mut known, &tx)
                    {
                        let _ = tx.send(Event::Error(format!("{e:#}")));
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        // ---- telemetry while a monitor is running
        if monitoring {
            if let Some(k) = &kb {
                let mut frames = Vec::new();
                let res = k.pump(&mut |t| frames.push(t));
                match res {
                    Ok(_) if !frames.is_empty() => {
                        let _ = tx.send(Event::Telemetry(frames));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let _ = tx.send(Event::Error(format!("device lost: {e:#}")));
                        kb = None;
                        monitoring = false;
                        let _ = tx.send(Event::Disconnected);
                    }
                }
            }
        }

        // ---- periodic rescan while idle
        if kb.is_none() && last_scan.elapsed() >= RESCAN_INTERVAL {
            last_scan = Instant::now();
            if api.refresh_devices().is_ok() {
                let found = enumerate(&api);
                if found != known {
                    known = found.clone();
                    let _ = tx.send(Event::Devices(found));
                }
            }
        }

        std::thread::sleep(IDLE_TICK);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle(
    cmd: Cmd,
    api: &mut HidApi,
    kb: &mut Option<Keyboard>,
    monitoring: &mut bool,
    known: &mut Vec<Candidate>,
    tx: &Sender<Event>,
) -> Result<()> {
    // Telemetry that arrives in the middle of a configuration exchange is
    // forwarded rather than dropped.
    let mut collected: Vec<KeyTelemetry> = Vec::new();
    let result = (|| -> Result<()> {
        let mut sink = |t: KeyTelemetry| collected.push(t);
        match cmd {
            Cmd::Shutdown => Ok(()),

            Cmd::Rescan => {
                api.refresh_devices()?;
                let found = enumerate(api);
                *known = found.clone();
                let _ = tx.send(Event::Devices(found));
                Ok(())
            }

            Cmd::Connect(c) => {
                api.refresh_devices().ok();
                let dev = Keyboard::open(api, &c)?;
                let snap = dev.read_snapshot(&mut sink)?;
                let label = format!(
                    "{} \u{2014} firmware {} \u{2014} {}-byte reports",
                    dev.candidate().display_name(),
                    snap.info.version_string(),
                    dev.packet_len()
                );
                *kb = Some(dev);
                *monitoring = false;
                let _ = tx.send(Event::Connected(Box::new(snap), label));
                Ok(())
            }

            Cmd::Disconnect => {
                if let Some(k) = kb.as_ref() {
                    let _ = k.stop_monitor();
                }
                *kb = None;
                *monitoring = false;
                let _ = tx.send(Event::Disconnected);
                Ok(())
            }

            Cmd::Reload => {
                let k = kb.as_ref().ok_or_else(|| anyhow::anyhow!("no device"))?;
                let snap = k.read_snapshot(&mut sink)?;
                let _ = tx.send(Event::Loaded(Box::new(snap)));
                Ok(())
            }

            Cmd::ApplyRt(rt) => {
                let k = kb.as_ref().ok_or_else(|| anyhow::anyhow!("no device"))?;
                let info = k.read_info(&mut sink)?;
                k.write_rt(&rt, info.rt_scale(), &mut sink)?;
                let _ = tx.send(Event::Status("Actuation applied".into()));
                Ok(())
            }

            Cmd::ApplyLed(led) => {
                let k = kb.as_ref().ok_or_else(|| anyhow::anyhow!("no device"))?;
                k.write_led(&led, &mut sink)?;
                let _ = tx.send(Event::Status("Lighting applied".into()));
                Ok(())
            }

            Cmd::ApplyCustomLed(colors) => {
                let k = kb.as_ref().ok_or_else(|| anyhow::anyhow!("no device"))?;
                k.write_custom_led(&colors, &mut sink)?;
                let _ = tx.send(Event::Status("Per-key colours applied".into()));
                Ok(())
            }

            Cmd::ApplyGameMode(gm) => {
                let k = kb.as_ref().ok_or_else(|| anyhow::anyhow!("no device"))?;
                k.write_game_mode(&gm, &mut sink)?;
                let _ = tx.send(Event::Status("Settings applied".into()));
                Ok(())
            }

            Cmd::ApplyKeys(keys) => {
                let k = kb.as_ref().ok_or_else(|| anyhow::anyhow!("no device"))?;
                k.write_keys(&keys, &mut sink)?;
                let _ = tx.send(Event::Status("Keymap applied".into()));
                Ok(())
            }

            Cmd::SetMonitor(on) => {
                let k = kb.as_ref().ok_or_else(|| anyhow::anyhow!("no device"))?;
                if on {
                    k.start_monitor()?;
                } else {
                    k.stop_monitor()?;
                }
                *monitoring = on;
                Ok(())
            }

            Cmd::SetCalibration(on) => {
                let k = kb.as_ref().ok_or_else(|| anyhow::anyhow!("no device"))?;
                if on {
                    k.start_calibration()?;
                    let _ = tx.send(Event::Status(
                        "Calibration running \u{2014} press every key to its bottom".into(),
                    ));
                } else {
                    k.stop_calibration()?;
                    let _ = tx.send(Event::Status("Calibration finished".into()));
                }
                *monitoring = on;
                Ok(())
            }

            Cmd::FactoryReset(scope) => {
                let k = kb.as_ref().ok_or_else(|| anyhow::anyhow!("no device"))?;
                k.factory_reset(scope)?;
                std::thread::sleep(Duration::from_millis(200));
                let snap = k.read_snapshot(&mut sink)?;
                let _ = tx.send(Event::Loaded(Box::new(snap)));
                let _ = tx.send(Event::Status("Reset complete".into()));
                Ok(())
            }
        }
    })();

    if !collected.is_empty() {
        let _ = tx.send(Event::Telemetry(collected));
    }
    result
}
