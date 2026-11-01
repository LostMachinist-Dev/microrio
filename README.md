# MicroRIO

A driver station for microcontrollers: live telemetry, control, charting and
logging for one board or several, over plain UDP on a local network.

Declare channels by name in the sketch. The station negotiates the schema once
on connect and both sides speak in numbered slots afterwards, so names never
ride in a telemetry or control frame — which is what lets a 2 KB board spend
its memory on your code instead of on channel names.

```
   board                      network                    station
 ──────────                  ─────────                 ───────────
 telemetry_f32("battery_v")  ──  UDP  ──▶   a reading on screen, in the chart,
 control_f32("drive_x")      ◀──  UDP  ──   a value you set, logged beside it
```

## What it is for

Driving and watching a machine while it runs: a rig on a bench, a robot, a
test stand. Everything on screen is live, everything is recorded at full rate,
and the recording is rectangular — one row per sample, every column filled.

## What it does

- **Any mix of boards.** AVR, ESP32, anything with an Arduino `UDP`. A node
  describes itself at connect; nothing in the station is specific to a make of
  board.
- **Safety that assumes failure.** Estop is global, enable is per node, and a
  UI that stops drawing faults every board and leaves it disabled. The board
  has its own watchdog and its own failstates, so it goes safe whether or not
  anyone is listening.
- **Telemetry you can trust as a record.** Samples are numbered; the station
  acknowledges what it has and the board resends what is missing while it
  still holds it. A dropped frame leaves no hole in the log. What truly never
  arrives is counted and reported rather than quietly interpolated.
- **Control that is never resent.** Full state every tick, last write wins. A
  stale command is worse than a dropped one.
- **One station per board, and watchers for the rest.** A node accepts control
  from one station. Any other station watches at full rate through that one —
  the board never sends to a station it is not already talking to, because
  that is what costs a board its timing. A watcher can set nothing, except the
  estop, which always works.
- **Charts** with up to fifteen minutes of history, drag to look back, pause,
  hover readout, and X-Y plots between any two channels on any two boards.
- **Joystick input** through any controller the system reports, conditioned
  per channel and overridable with a fixed value at any moment.
- **Cross-node links.** One board's reading drives another board's output,
  scaled and limited; a source that goes quiet faults the destination rather
  than holding a stale value.
- **Logging** at full rate to a binary log, exported to CSV for the whole
  session or a marked window.

## Getting started

1. Install the station. On Linux, `sudo dpkg -i microrio-station_*.deb`; on
   Windows, run `MicroRIO-Setup-*.exe`.
2. Install the Arduino library: unzip `MicroRIO-*.zip` into your sketchbook's
   `libraries` folder.
3. Flash `examples/BasicNode` with your board's address, open the station, and
   put that address in Settings → Nodes.

`RUNNING.md` is the operator's manual — what each page does, what to do when
something is wrong. `DESIGN.md` is why it is built this way.

## Status

Version 0.1.0. It drives real hardware every day on the bench it was built
for, and the parts that have been tested are listed as tested because nothing
here is simulated.

**Proven on real boards.** An Arduino Mega over Ethernet and an Olimex
ESP32-POE-ISO over WiFi: 50 conformance checks of the C++ codec against the
Rust one on each board, telemetry at 50 Hz with 0.0% loss, deliberate frame
drops recovered by backfill, one board's reading driving the other's output
end to end, failstates on a tripped watchdog, clock sync, full-rate logging
exported to CSV, and two stations against both boards at once — one driving,
one watching through it. 102 automated tests cover the codec and the
station's logic.

**Known limitations.**

- The Windows build is cross-compiled and has been exercised under wine, not
  on a Windows machine. Wine is not Windows.
- An Uno is in the compile matrix and its memory use is measured, but no Uno
  has been on the network. It gets loss measurement rather than backfill: a
  2 KB board keeps one record, not eight.
- The ESP32 PoE sketch compiles and passes conformance over USB; no Ethernet
  cable has been connected to that board.
- Three or more nodes at once has not been tried. Nothing in the design
  limits it and nothing has confirmed it.
- A watched node shows no round-trip time, because a watching station never
  speaks to the board, and it goes blank if the station driving it closes.
- The subnet broadcast used to reach watchers assumes a /24.
- One rate per node, set by the station. It governs how often the board
  *reports*, never how fast its loop runs.

## Repository layout

| Path | What it is |
|---|---|
| `spec/protocol.toml` | The wire protocol. The single source of truth: the Rust constants and the C++ header are generated from it |
| `microrio-proto/` | The codec. `no_std`-capable, no I/O, shared by the station and the tools |
| `microrio-station/` | The driver station |
| `arduino/MicroRIO/` | The Arduino library. Written here, published to [microrio-arduino](https://github.com/LostMachinist-Dev/microrio-arduino) by `arduino/publish-library.sh` |
| `xtask/` | `gen-header`, `conformance <port>`, `probe <ip>` |
| `packaging/` | Installers |

## Building

```sh
cargo test --workspace          # the codec, the station's logic
cargo run -p microrio-station   # the station
cargo xtask gen-header          # regenerate the C++ header from the spec
packaging/release.sh            # everything, into ~/deliverables/microrio/
```

Hardware is tested for real rather than simulated. With a board on USB:

```sh
cargo xtask conformance /dev/ttyACM0   # 38 checks of the C++ codec against the Rust one
cargo xtask probe 192.168.0.50         # the live link: rate, loss, backfill, watchdog, clock
```

## Licence

MIT or Apache-2.0, at your option. It drives machinery and comes with no
warranty; whatever you connect it to is yours to make safe.
