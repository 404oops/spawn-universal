# SPAWN Universal

A free-software configurator for SONiX-based **SPAWN magnetic (Hall-effect) keyboards**,
written in Rust with [GPUI](https://gpui.rs). Native on **Linux, macOS and Windows** —
no Electron, no bundled Chromium, no telemetry, no auto-updater phoning home.

The stock software is a Windows-only Electron app. This is a clean reimplementation of
the keyboard's HID protocol so the hardware you bought stays usable on the OS you choose.

> **Status:** the transport and every read path are verified against real hardware
> (SPAWN `0C45:8A01`, firmware 1.09). See [Verification](#verification) for exactly what
> has and has not been exercised on a device.

---

## Features

| | |
|---|---|
| **Actuation** | Per-key trigger depth (0.10–3.40 mm), independent press/release rapid-trigger sensitivity, continuous-RT and rampage modes, per-key switch profile |
| **Keys** | Remap any key to another key, modifier, mouse button or media control |
| **Lighting** | Effect mode, brightness, speed, primary/secondary colour, per-key painting |
| **Settings** | Polling rate (1000/4000/8000 Hz), top and bottom dead zones, stability mode, auto-calibration |
| **Calibration** | Guided calibration so the board learns each switch's real travel range |
| **Live view** | Real-time per-key travel, straight from the firmware's telemetry stream |

Select keys on the board to scope an edit; with nothing selected, actuation and lighting
edits apply to the whole keyboard. Remapping always requires an explicit selection.

## Install

Requires Rust 1.85 or newer.

```bash
git clone https://github.com/oops404/spawn-universal
cd spawn-universal
cargo build --release
./target/release/spawn-universal
```

### Linux

hidraw nodes are root-only by default. Install the packaged udev rule once:

```bash
sudo cp packaging/99-spawn.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then unplug and replug the keyboard. Build dependencies on Debian/Ubuntu:

```bash
sudo apt install libudev-dev libwayland-dev libxkbcommon-x11-dev libvulkan-dev cmake clang
```

### macOS and Windows

No setup needed. On Windows, close the vendor software first — it holds the device open.

## Command line

```bash
spawn-universal                  # open the GUI
spawn-universal --list           # every HID interface, and which one is usable
spawn-universal --probe          # read-only dump of the current configuration
spawn-universal --selftest       # scan, connect and read back, as the GUI does
spawn-universal --test-lighting  # write test; restores the previous state
spawn-universal --version
```

Only one program can hold the keyboard at a time. If a second copy (or the vendor
software) has it open, the app says so — use **Release** in the sidebar to hand it back.

`--list` is the first thing to run when a keyboard is not detected.

---

## The protocol

Recovered from the vendor application for interoperability, and reimplemented from
scratch in [`crates/spawn-protocol`](crates/spawn-protocol). No vendor code, assets or
strings are used or redistributed here.

**Transport.** Vendor HID reports on usage pages `0xFF67`/`0xFF68` (the boot-keyboard
interface on the same device must never be used for configuration). Report ID `0`.
Reports are 32 or 64 bytes, read from the HID report descriptor.

**Request** (host → device):

```
[0]     0xAA                       magic
[1]     command
[2]     payload length in this packet, or a command-specific scalar
[3..5]  little-endian address of this chunk
[5..8]  up to three command-specific header bytes;
        when absent, byte 6 carries the "final chunk" flag
[8..]   payload
```

**Response** (device → host): identical, but starting with `0x55`.

Every packet reserves 8 bytes of header, so a 64-byte report carries 56 payload bytes.
Transfers longer than one packet are split into chunks and **each chunk is acknowledged
before the next is sent**; the reply echoes the address, which makes reassembly verifiable.
Unsolicited calibration frames (`0xFB`) can arrive at any time, including mid-transfer,
and are routed to the telemetry stream rather than mistaken for a reply.

**Key records** are 4 bytes per slot across 126 slots: a page selector plus three
arguments. Page `0` means *use the factory default*, so an unconfigured keyboard reads
back as all zeros — the app substitutes sane defaults rather than writing `0.00 mm`
actuation back to the board.

**Rapid-trigger records** are 8 bytes per slot: switch profile, flags, trigger depth
(hundredths of a millimetre) and press/release deltas. The delta scale is `0.01 mm` or
`0.001 mm` depending on the `rt_precision` byte in the device info block.

A full command table is in [`lib.rs`](crates/spawn-protocol/src/lib.rs).

## Verification

Confirmed against real hardware — SPAWN `0C45:8A01`, firmware 1.09.

**Reads** (`--probe`, read-only):

- device enumeration and configuration-interface selection
- report-descriptor parsing (correctly detected 64-byte reports)
- chunked reads with address echo across 18-packet transfers
- device info, game mode, LED effect, per-key LED, keymap and rapid-trigger reads
- graceful timeout on a command this firmware does not implement (`0x1F`)

**Writes** (`--test-lighting`, driven through the same worker the GUI uses, and
confirmed by eye on the keyboard):

- effect changes apply: static, breathing and spectrum all render correctly
- colour applies: solid red, green and blue each land as asked
- `SET_CUSTOM_LED_DATA` stores per-key colour and reads back exactly

### Factory defaults

Recorded from a full factory reset (`--record-defaults`) on `0C45:8A01` firmware 1.09,
and pinned in `GameMode::factory_default()` and `LedEffect::factory_default()`:

| | |
|---|---|
| polling rate | code 6 — 8000 Hz |
| dead zones | 0.00 mm top and bottom |
| stability mode | on |
| automatic calibration | on |
| sleep | 1 minute |
| lighting | effect 11 (Waves), brightness 5, speed 4, `color_mode` 1 |
| rapid trigger | **every slot zeroed** |
| keymap and per-key colour | entirely unset |

The last two matter: a reset board stores nothing at all for actuation, so "unconfigured"
is the reset state rather than a fault, and the app substitutes its own defaults for
display instead of writing 0.00 mm back.

Firmware quirks worth knowing, all established by measurement rather than assumption:

| | |
|---|---|
| `color_mode` | **0** = use the RGB in the record, **1** = firmware picks its own (rainbow) and ignores the colour entirely. Choosing a colour must also set 0, or nothing happens. |
| effect list | only **1–19** and **128** are valid here. The vendor adds 23/24/25/254 only when the board's config names them under `customEffect`; the SPAWN has no lighting config, so selecting one of those leaves the keyboard dark. |
| `mode` 0 | backlight off — a state, not an effect |
| `report_rate` | a code, not an index: 3=1000, 4=2000, 5=4000, 6=8000 Hz |
| dead zones | 0–0.5 mm, not 0–1 |
| rapid-trigger ceiling | comes from the selected switch: 3.4 mm for Axis 6/7/10/20, 3.3 mm for Jade and King |
| switch profiles | the config's `axis_6`, `jade` and so on are translation keys, not names. The switches are Graywood/Yingyi/Yongchun (1), Black Emperor (4), Spirit Cloud (5), Neptune (6), Magnetic Jade Pro (2) and Magneto (3) |
| direction | 0 right, 1 left, 2 up, 3 down — and an effect offers only the pair matching the axis it travels along, never all four |
| mouse actions | function byte 1 is a button (left 1, right 2, middle 4, back 8, forward 16), 3 is the wheel, and scroll down is 255 |
| `brightness` / `speed` | both 1–5, not 0–255. The vendor reads the bounds as `minSpeed \|\| 1` / `maxSpeed \|\| 5` and this board declares no override; values above 5 are misread rather than faster |
| `driver_setting` | written as `0xFF`, always reads back `0` |
| colour read-back | `GET_LED_EFFECT` does not reliably echo the colour that was written; trust it for mode, brightness, speed and direction only |
| `GET_ALL_LIGHTS_RGB` | not implemented on this firmware — returns 126 zeroed entries rather than timing out |
| zeroed records | mean *unconfigured*, not "0.00 mm" — the app substitutes defaults |

Still not exercised on hardware: actuation, keymap and settings writes, calibration, and
the live telemetry stream. They share framing with the verified paths and are unit-tested,
but treat the first write as a test and keep `--probe` output beforehand.

A note on method: several of these were found only by changing a value and *looking at the
keyboard*. Read-back alone would have confirmed the wrong conclusion more than once — the
colour bytes appear to be ignored if you only check what the device echoes.

## Layout

```
crates/spawn-protocol/   pure protocol: framing, records, layout. No I/O, heavily tested.
crates/spawn-app/        the application: hidapi transport, background device thread, views.
packaging/               udev rules for Linux.
```

The interface is built with [Vampir](https://crates.io/crates/vampir), a GPUI widget
toolkit: surfaces are lit from above, controls you press are raised and things you drag
into are recessed. The app supplies a `ControlState`, implements `ControlHost`, and pumps
slider drags from its root mouse handlers.

Everything comes from crates.io — Vampir builds on `gpui-ce`, the published release of
GPUI, so there is no git dependency. The workspace uses edition 2024.

One dependency detail worth knowing: `gpui_ce_platform`'s font backend is behind its
`font-kit` feature, and it is not on by default. Without it the window lays out perfectly
and draws no text whatsoever, which is a confusing thing to debug — so the app checks at
startup and logs an error if no fonts resolved.

`spawn-protocol` performs no I/O and forbids `unsafe`, so every framing and encoding rule
is directly unit-testable — 33 tests cover chunk boundaries, endianness, header-slot
contention, precision scaling and malformed input. The application adds headless GPUI
render tests that draw every tab, so a broken layout fails in CI rather than on a desktop;
they run against an offline worker and never touch real hardware.

## Packaging

```bash
packaging/macos.sh --dmg       # dist/Spawn Universal.app, and a disk image
packaging/linux.sh --install   # into ~/.local, with a .desktop entry
sudo packaging/linux.sh --system   # into /usr/local, with the udev rule
powershell -File packaging\windows.ps1 -Zip   # a folder and a zip
```

Tagging `v0.1.0` runs [the release workflow](.github/workflows/release.yml), which
builds an AppImage, a `.deb` and an `.rpm` on Linux, an NSIS installer on Windows and a
disk image on macOS, then drafts a release with them attached. A manual run builds the
same set without publishing, for checking a packaging change without cutting a version.

The AppImage is built with `linuxdeploy` rather than assembled by hand, and the workflow
checks the result answers the runtime's own flags (`--appimage-help`, `--appimage-offset`,
`--appimage-extract`) and has the layout the specification requires.

The icon is drawn by [`packaging/make-icon.py`](packaging/make-icon.py) rather than kept
as a binary, so the source of truth is readable and every size is rendered rather than
resampled. The mark is a keycap with its actuation point beneath it.

It is deliberately **not** the manufacturer's logo. That artwork is theirs, this project
is GPL, and an application wearing their mark would imply it is their software — which
the notice at the end of this file expressly denies.

## Contributing

Other SONiX boards use this same protocol with different layouts. Adding one means a new
table in [`layout.rs`](crates/spawn-protocol/src/layout.rs) and its USB product ID —
`--list` and `--probe` will tell you what you need. Layout tests assert that every row
sums to the same width, so a mistyped key size fails CI rather than rendering crooked.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).

Free software: you may use, study, share and improve it. Derivative works must carry the
same freedoms.

Not affiliated with or endorsed by the keyboard's manufacturer. Reverse engineering was
performed for interoperability — to make hardware work with operating systems its vendor
does not support.
