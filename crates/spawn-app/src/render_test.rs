// SPDX-License-Identifier: GPL-3.0-or-later
//
// Headless render checks. These build the real view and draw it through
// GPUI's test platform, so a missing font, a bad element id or a panic in a
// tab body fails here instead of on a user's desktop.

#![cfg(test)]

use crate::app::{Pane, Section, SpawnApp};
use gpui::TestAppContext;

#[gpui::test]
async fn every_section_and_pane_renders(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, cx| SpawnApp::offline(cx));

    for section in [Section::Keyboard, Section::Settings] {
        window
            .update(cx, |view, _, cx| {
                view.set_section(section);
                cx.notify();
            })
            .expect("view is alive");
        cx.run_until_parked();
    }

    // Every pane draws under the board.
    window
        .update(cx, |view, _, cx| {
            view.set_section(Section::Keyboard);
            cx.notify();
        })
        .expect("view is alive");
    for pane in [Pane::Actuation, Pane::Keys, Pane::Lighting] {
        window
            .update(cx, |view, _, cx| {
                view.set_pane(pane);
                cx.notify();
            })
            .expect("view is alive");
        cx.run_until_parked();
    }
}

#[gpui::test]
async fn selecting_keys_then_editing_does_not_panic(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, cx| SpawnApp::offline(cx));

    window
        .update(cx, |view, _, cx| {
            view.select_all();
            view.nudge_trigger(0.05);
            cx.notify();
        })
        .expect("view is alive");
    cx.run_until_parked();

    let trigger = window
        .update(cx, |view, _, _| view.probe_trigger_mm())
        .expect("view is alive");
    assert!(trigger > 0.0, "actuation must never be presented as zero");
}
