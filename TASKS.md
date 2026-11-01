# MicroRIO — Tasks

*Renamed from TOE on 2026-10-24. Successor to the 2026 TOE archive.*

## Done
- Extracted and read the 2026 `TOE archive.zip`; wrote `MicroRIO-origins.md`.
- Copied sources to `archive/`, redacted the hardcoded WiFi password.
- Wrote `DESIGN.md`; revised to rev 2 after review.
- Surveyed hardware, measured real AVR memory budget — `HARDWARE.md`.
- Renamed project and deliverables folder to `microrio`.
- **All eight build steps from `DESIGN.md` §13, through to a board on Ethernet.**

## Settled
- **MicroRIO**: Rust driver station (Linux + Windows), Arduino C++ firmware library.
- Multi-node, manual IP entry, saved TOML profiles.
- Named keys negotiated once via schema hash; numeric slots on the wire.
- **One rate per node.** No rate groups — rectangular logs matter more than the bandwidth.
- **UI loss is a fault.** Three paths to neutral: UI heartbeat stale, controller watchdog,
  operator estop. Station keeps transmitting FAULT rather than going silent.
- Direction is explicit: `telemetry_*` is displayed on the GUI, `control_*` is populated by it.
- Compile-time capacity; streaming encode; names in PROGMEM. Uno runs the large config in 927 B.
- egui GUI; gilrs joystick with a live validation panel that works with no node connected.
- **Real hardware only. No simulator.**

## Build steps (`DESIGN.md` §13)
1. ~~`spec/protocol.toml` + `microrio-proto` codec, roundtrip tests.~~ **DONE**
2. ~~`arduino/MicroRIO` library.~~ **DONE** — 26/26 conformance on Mega and ESP32.
3. ~~`microrio-station` core: handshake, control tick, fault logic.~~ **DONE**
4. ~~Logger + CSV export.~~ **DONE**
5. ~~egui UI.~~ **DONE**
6. ~~Joystick panel.~~ **DONE** — live axis/button view, mapping editor.
7. ~~ESP32 support.~~ **DONE** — core installed, sketch compiles, conformance passes.
8. ~~A board on Ethernet, end to end.~~ **DONE** — Mega live at 192.168.0.50.

## Verified
- 24 codec tests (proptest roundtrips, truncation rejection, schema hashing, checked RTT).
- 26 hardware conformance checks over serial, on **both** `arduino:avr:mega` and
  `esp32:esp32:esp32-poe-iso`, three consecutive runs each.
- 2 logger/CSV tests.
- Uno compiles at 396 B SRAM (19%) with 11 channels — the memory canary holds.
- Station app builds and runs on Linux; Windows `.exe` cross-compiles via
  `x86_64-pc-windows-gnu`.
- **End to end over Ethernet against the Mega**: schema negotiated, 50 Hz both ways,
  rx/tx symmetric with no loss, node obeys ENABLED, node faults itself when control
  stops. 6 probe checks pass (`cargo xtask probe 192.168.0.50`).
- CSV export of a real 50 s session: 2500 rows = exactly 50 Hz, rectangular.
- **Driving while ENABLED — confirmed by hand on 2026-10-25**, from the
  Windows build: an output moves when enabled and does not when disabled.
- 52 tests: 24 codec, 7 chart/history, 6 control-path, 4 logger/CSV, 7 mapping,
  4 device match.
- **Operator revision re-verified live against the Mega**: 1197 rows over
  23.94 s = 49.95 Hz, rectangular, every column populated; all 6 probe checks
  including clock sync still pass.
- **Joystick end to end**: a DualShock 4 (`054c:09cc`, the same pad as the 2026
  archive) seen by gilrs as "PS4 Controller". A LeftStickX -> drive_x mapping with
  trim 0.5 produced exactly 0.50 in the UI and `ctrl_drive_x = 0.5` in the logged
  CSV, proving gilrs -> conditioning -> intent -> control frame -> board.
- 7 mapping-maths tests, including the real -0.008 resting drift this pad shows.
- **Clock sync over UDP**: probe skews the node's RTC by an hour, confirms it
  took, then puts it back on station time. Both checks pass.
- RTC confirmed present on the Mega: 25.25 C, holding UTC.

## GUI rebuild (2026-10-24)
- Restyled to the 2026 instrument look: palette and trace colours lifted from
  `archive/giga-gui/public/well developed gui.txt` and `6Achannels.ino`.
  Bahnschrift/DIN are Windows fonts and egui needs font bytes, so the feel comes
  from monospaced numerics and spaced uppercase headers instead.
- Telemetry grid: raw and scaled side by side, editable slope/offset/units,
  per-row plot arming lamp.
- Control grid: `[- value +]` with step, min, max, offset, and a wire column
  showing what actually goes out after offset and clamp.
- Chart tab with timebase, auto/manual Y, legend, scrolling "seconds ago" axis.
  Traces are scaled values so slope doubles as axis fitting.
- Clock tab: station local/UTC/unix against node RTC, drift, and a sync button.
- `SimNode` sketch: sine, ramp, square, noise, DS3231 RTC on the Mega's I2C
  (20/21), and clock sync via `set_unix` + `sync_time`.

## Profile handling (2026-10-24)
- Open / Save / Save as… / Reload in a bar under the safety row, using native
  file dialogs (`rfd`). A profile is an ordinary file anywhere on disk, so one
  build serves several machines.
- Loading a profile now **rebuilds the links immediately**. A supervisor task
  owns the per-node tasks and respawns them when the profile epoch moves, which
  also removes the old "node changes take effect on restart" wart.
- Opening a profile always leaves the station disabled.
- Joystick device matching is tolerant across platforms: exact, then
  case-insensitive, then a single-pad/single-entry assumption. Ambiguity does
  nothing rather than driving the wrong channel.

## Packaging (2026-10-25)
- `arduino/package-library.sh` builds `MicroRIO-<ver>.zip` in the Arduino
  sketchbook layout. Regenerates the protocol header first so the zip can never
  ship a header that has drifted from `spec/protocol.toml`.
- Verified the real way: extracted into `~/Arduino/libraries/`, `arduino-cli
  lib list` sees it, and both examples compile with no `--library` flag.
- Library is header-only (template), plus `keywords.txt` and a README.
- Station installed as a Linux desktop app: `~/.local/bin/microrio-station`,
  `.desktop` entry with Chart and Joystick actions, SVG icon in the project
  palette. Needed no sudo. Default profile seeded to the Mega with the
  waveforms armed.

## Measurement (2026-10-25)
- **battery_v was reading a floating pin.** Two unconnected pins settle to
  nearly the same voltage (580 and 572 counts) because they share a package,
  not because the ADC carries charge between channels. It is now a simulated
  pack voltage sagging under the sim's own load. `a0` stays a real ADC read,
  documented as floating unless wired.
- **Frame delivery is now measured**, counted from gaps in the node's sequence
  numbers, in both the station and the probe. Two stations fighting over one
  board presents as packet loss and had sent me to the network twice; the
  station now names that as the likely cause. Verified with a competing
  station: 100% delivered alone, 26% with a competitor.
- **UDP vs TCP benchmarked on the W5100** (DESIGN.md appendix). UDP 596 us per
  frame with no variation; TCP 562-987 us per write plus a 1.4-4 ms flush. TCP
  is not slower on average, it is less deterministic — and a retransmitted
  control frame is a stale one. UDP stays, and is not made selectable.
- ufw is active on the box; the station works through it because it speaks
  first.

## Operator revision (2026-10-25)
From the operator's review after driving a real output from the Windows build.
- **Layout built for two window shapes.** Side by side above 1180 px, stacked
  below it. The chart takes whatever height the grids do not need, measured
  from the last frame rather than a fixed fraction. `--size WxH` reproduces
  either shape. Verified by screenshot at 1920x1080 and 960x1080.
- **Operator page** carries telemetry, control and a chart at once. The Charts
  tab holds that chart plus any others.
- **Charts name their axes** instead of arming channels: X and Y are both
  picked from the same list (elapsed time, clock time, or any channel on any
  node), so flip is a plain swap and X-Y plots fall out for free. Cross-node
  traces are matched by timestamp with a zero-order hold; a stale source
  leaves a gap. Old `plot = true` profiles migrate into the first chart.
- **One time zone** (`system` or IANA), used by every clock, chart axis and
  CSV column. No UTC anywhere in the window. `system` resolves to a real IANA
  name so it can say `MST` rather than `-07:00`.
- **Logging has a start and a stop.** The background log still runs the whole
  session; start/stop marks a window and stopping asks where to save a CSV of
  just that window. Logs now carry the wall clock at session start, so rows
  get a `time_local` column.
- **Enable is per node, estop is global.** Enable is runtime-only and never
  loaded from a file. `NodeConfig.enabled` renamed `link_active` (serde alias
  keeps old profiles working).
- **A UI stall no longer clears itself** — it disables every node and says so
  until dismissed. Forced by the file dialogs, which block rendering.
- **Cross-node links**: telemetry on one board drives a control channel on
  another, scaled and clamped. A source quiet for 200 ms faults the
  destination rather than holding the last value.
- **Settings page made readable**: titled sections, real field widths.
- Rate box moved onto the node row and labelled honestly: it is the station's
  tx rate, shown next to the node's own advertised rate, orange when they
  disagree. It now takes effect without rebuilding the link.

## Fixed
- **The control `wire` column was lying.** It displayed `clamp(value + offset)`
  while `build_values` sent the raw intent — the offset never reached the
  frame. Now applied in one place that both the grid and the frame use.
- Opening straight to the Charts tab showed "No charts yet": the first chart
  was created by the Operator page, which had never drawn. Created centrally
  now, as soon as a node reports its schema.
- Making that real would have broken clock sync: the i32 default clamped to
  ±10 000 and `set_unix` carries ~1.8e9. Integer channels now default to the
  full i32 range. The probe would **not** have caught this, because it does
  not go through the station's control path; there is a test that does.

## Final polish (2026-10-31)
From `MicroRIO-design.md` review, items 1-6.
- **Done** — protocol v2: numbered telemetry records, station ack in every
  control frame, backfill on Mega/ESP32, rate field in control (telemetry and
  logging rate only, never the board loop). Spec regenerates both sides.
- **Done** — conformance 35/35 on Mega and ESP32 (backfill, ack, rate checks
  added). Probe over Ethernet: 17 frames dropped on purpose, 17 records
  recovered, 0 lost; 25 Hz request obeyed.
- **Done** — Uno: no resend buffer, streams as before. +27 B SRAM for the
  record counter, ack and loss/latency counters (BasicNode 395 -> 422 B).
  Mega gains ~450 B of resend buffer, sized to what one frame can carry.
- **Done** — big on-screen e-stop, no hotkey. Value-label tables (status text
  / dropdown, raw number when unmapped). Pins in the node header (cap 5,
  pinned groups count), warn-only thresholds. Groups with drag order,
  collapse, pin; a control lives in one group only.
- **Done** — 20 s live session exported: 1089 rows, 49.96 Hz, rectangular.
- **Done** — UI review round 1 (2026-10-31): Save/Reload/Disable-all gone;
  single enable toggle; monochrome text; groups carry full rows with bigger
  controls; value beside wire; plot is a checkbox; set_unix/sync_time hidden;
  operator chart separate from Charts tab; 10 s-15 min windows, pause, clear,
  hover readout; joystick routes into the control row (expo/range/offset
  there, live value shown, hold override); analog triggers read; echo
  confirm/warn; per-node comms watchdog alarm; library `failstate()` and
  `watchdogOK()`; Clock tab copes with no RTC; time zone shows offset + DST.
- **Done** — ESP32 on WiFi (`Esp32WifiNode`, `arduino/flash-wifi-node.sh`;
  credentials in `~/.config/microrio/wifi.txt`, never in the project). All
  probe checks pass over WiFi; conformance 38/38 on Mega and ESP32. Both
  boards logging together from the installed app.
- **Done** — UI review round 2 (2026-11-01): version + commit in the window
  corner and `--version` (build.rs stamps it); chart sets its own bounds every
  frame with zoom/scroll/boxed-zoom off and drag-to-pan plus a live button —
  the view can no longer be left unreadable; flip gone; every numeric field is
  typed, never dragged; telemetry grouped per node with cross-node
  pass-through last; Settings channel display per node; joystick lists
  whatever a device reports, raw codes included, so it is not tied to one make.
- **Done** — **Cross-node links proven on two real boards.** Mega `sine` →
  ESP32 `drive_x`, echoed back as `follow`: 1245 rows compared, mean error
  0.0018, max 0.045 (one frame at 50 Hz). Both boards log together at 50 Hz.
- **Done** — UI review round 3 (2026-11-01): **file dialogs no longer stop the
  machine** — Open, Save as and CSV export run the dialog on a worker thread,
  so the UI keeps drawing and nothing is disabled; Settings shows each
  channel's live value beside its settings and can invert a bool's lamp
  (`rtc_ok` lights on false); groups sit two across on a wide window; units
  are a label, not a box; control is one output listed per node rather than a
  section per board; charts can draw Y downwards; About panel with version,
  commit, protocol, licence, source and paths.
- **Done** — Clock tab lists every node with its own clock, drift and sync, plus
  sync all. A board with no RTC says so in its own row (the ESP32 does).
- **Done** — **Two stations on one node is fixed, not just reported.** Node-side
  ownership with a lease, refusals carrying a conflict flag in a spare bit (no
  version bump), station-side detection that works against old firmware too, a
  single-instance lock, and the probe says when a node belongs to someone else.
  Verified with two stations against the live Mega: the owner holds 0.0% loss
  where both used to see about half. 44/44 conformance on Mega and ESP32.
- **Done (rebuilt 2026-11-01)** — **Watchers are fed by the owner's station,
  not by the board.** The first attempt had the board send to watchers and it
  halved the machine's rate: a W5100 resolves the address before every packet,
  and a watcher on a sleepy WiFi laptop cost **377 ms per frame** — worst loop
  2.02 s, 50 Hz down to 4.4 Hz. Now the owner forwards each frame (and the
  schema every 2 s) over one UDP port; a watcher sends the board nothing at
  all except an estop, which the node still obeys over its owner. Measured
  after: board loop 3.4 ms, both stations at 50 Hz, 0.0% loss each, watcher
  records a complete log of its own. 50/50 conformance on Mega and ESP32.
- **Superseded** — the first watcher design: It marks its
  frames view-only: the node answers it with full-rate telemetry, never takes
  a value from it, and obeys its estop (held 2 s, owner cannot override).
  The station drops to watching on its own, shows VIEW ONLY and a take-control
  button, greys the controls, and stops logging control it is not sending.
  `--view` starts a watcher deliberately. Watcher copies share the owner's
  sequence number, so nobody sees phantom loss. 50/50 conformance on Mega and
  ESP32; verified live with two stations — commander 0.0% loss, watcher 0.1%.
- **Done** — Windows installer built and tested under wine: silent install,
  four Start-menu shortcuts, the app runs, uninstaller removes everything.
- **Next** — the fourth UI pass.
- **Next** — DESIGN.md: enable per node in sections 2/4, station speaks first
  (SCHEMA_REQ) in section 5, fold the appendix in, protocol v2. READMEs.
- **Next** — Windows installer (needs `sudo apt install nsis`) and Linux .deb.
- **Next** — tidy history, then wait for "publish" (two repos).

## Not yet tested
- **The ESP32's Ethernet path is unexercised.** It runs on WiFi (`Esp32WifiNode`,
  proven on the network); `Esp32PoeNode` compiles and passes serial conformance,
  but no cable has been plugged into it.
- **Start/stop logging has not been clicked by hand** — the windowed export is
  covered by tests and the whole-session export was run against a real 24 s
  session, but the Save dialog path itself is untested by a human.
- Windows: the `.exe` builds but has not been run on a Windows machine.

## Notes
- **Published 2026-11-01** as two public repos in `LostMachinist-Dev`:
  [microrio](https://github.com/LostMachinist-Dev/microrio) (station) and
  [microrio-arduino](https://github.com/LostMachinist-Dev/microrio-arduino) (the library).
  GitHub sees one root commit each: none of the working history is public. The full local
  history is on `pre-publish-backup` and `master`. The published tree has the home WiFi
  name and an old GitHub handle replaced with placeholders. Later work goes on top of
  `main` as ordinary commits — never a force-push. The library is written in
  `arduino/MicroRIO` and pushed to its own repo by `arduino/publish-library.sh`; never
  edit the published copy.
- A node **answers to one station at a time and enforces it**: the first to send control
  owns it, others are refused and told so, and it frees up a watchdog period after the
  owner goes quiet. Schema requests are always answered. The station also refuses to
  start twice on one machine (`--allow-multiple` overrides).
- **`cargo xtask conformance` leaves the board holding `Conformance.ino`**, which has no
  network at all. Every station page then says it is waiting for a schema, which looks
  exactly like a broken station. Reflash the node sketch afterwards:
  `arduino-cli compile -b arduino:avr:mega --library arduino/MicroRIO -u -p /dev/ttyACM0 arduino/MicroRIO/examples/SimNode`
  and `arduino/flash-wifi-node.sh /dev/ttyUSB0`. The command now prints both lines when it
  finishes, and the station names the three things to check rather than only saying it waits.
- `enp195s0` has carrier but no DHCP lease (link-local only). The Mega reaches the box
  via the router over WiFi instead, which works but adds latency.
- Mega currently holds `SimNode` (with `mode`/`mode_echo`) at 192.168.0.50; probe clean on 2026-10-31.
- ESP32 holds `Esp32WifiNode` (waveforms + `follow`) at 192.168.0.51 on WiFi.
- Live profile carries both nodes and the sine → drive_x cross-node link.
- Profile backup before the demo groups were added: `~/.config/microrio/station.toml.bak-20261031`.
