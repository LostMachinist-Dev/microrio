# MicroRIO — origins (the 2026 "TOE" archive)

> This document describes the **original 2026 project**, which was called TOE
> (Telemetry Over Ethernet). The successor being built now is **MicroRIO**.
> TOE naming is kept throughout below because that is genuinely what the archive
> was called — the sketches really do `#include <TelemOverETH.h>`.

## What this project was trying to be

*Reconstructed from `TOE archive.zip` (59 source files, Jan 8 2026 – Apr 19 2026). Nothing in the archive states the goal outright; this is pieced together from the code, the file names, the one `read me.txt`, and the order the revisions were saved in.*

---

## The one-sentence version

**TOE = Telemetry Over Ethernet.** You were building a reusable Arduino library plus a PC-side GUI stack so that *any* Arduino project could get a live, two-way, instrument-grade link to a browser dashboard — telemetry out, controls in, joystick in, clock synced, health monitored — by adding about five lines to a sketch.

The name is confirmed by the newest file in the archive, `TOE2.0_20260415_1931.ino`, which does `#include <TelemOverETH.h>`.

---

## What "done" was supposed to look like

The final sketch in the archive is the clearest statement of intent you ever wrote, because it's written against a library that **does not exist in the archive** — it's the target API, written as if the work were finished:

```cpp
#include <TelemOverETH.h>
TelemOverETH telem;

void setup() {
    telem.ipsetup(mac, ip, gateway, subnet, dns);
    telem.freq(10);          // 10 Hz telemetry
    telem.max_val(128);      // plenty of keys
    telem.begin();
}

void loop() {
    telem.loop();
    telem.write("a0", analogRead(A0));        // telemetry out, by name
    digitalWrite(LED_BUILTIN, telem.readBool("led"));  // control in, by name
    telem.write("loop_pct", telem.loopDuty()); // self-instrumentation
}
```

That's the goal: **five lines of setup, then `write()` and `read()` by name.** Everything else in the archive is you working backwards toward that.

---

## The five things it had to do

Every revision, from the first file to the last, keeps these five concerns. They're the actual requirements:

1. **Telemetry out** — board → PC, continuously, at a fixed rate (25 Hz early, 10 Hz later).
2. **Control in** — PC → board, same channel, same cadence. Not just a web page with buttons; a real control path.
3. **Joystick in** — a physical HID game controller on the PC driving the board. This is the one that ate the project.
4. **Time** — a DS3231 RTC on the board, and operator-initiated time sync from the PC (`sync_time` / `unix_set` in the final sketch).
5. **Health / self-instrumentation** — link status, latency, loop duty cycle, free RAM, packet counters, RTC OK, Ethernet OK, firmware version. You wanted the link to tell you about itself.

Point 5 is unusually consistent and is the thing that marks this as an instrumentation project rather than a hobby dashboard. `DSV_3.0.0` has a whole `HealthKey` enum: `wifiConnected`, `serverReachable`, `timeSynced`, `wifiReconnectAttempts`, `controlAgeMs`, `latencyMicros`, `clockDriftMicros`, `cpuLoadPercent`.

---

## How it evolved — six phases

### Phase 1 — Web oscilloscope on WiFi (Jan 8–9)
`6Achannels`, `MoreThanGraphing`, `GraphButtonsSerialPrint`, `SuperBasicGUI`, `APExample`, `BoardBlink`

GIGA R1 WiFi serves its own HTML page over HTTP and streams JSON at `/data`. Header comment: *"6-channel Web Oscilloscope, Dark theme + Bahnschrift/D-DIN font, Keysight-style channel tabs, Y-axis manual scaling + auto-scale."*

The visual target was **Keysight test equipment**. That aesthetic — dark, monospace, instrument-grade — survives all the way to the end.

Controls appear almost immediately (`/led?c=R|G|B`, `/set?value=X`), so it was never meant to be read-only.

### Phase 2 — Joystick enters, HTTP breaks (Jan 9–10)
`GUIwJoy_`, `SoloJoy`, `2.3.7/`

`GUIwJoy_` adds `joyAxis[8]` and `joyButtons[22]`. `SoloJoy` switches to WebSockets (`ArduinoWebsockets`) and introduces **burst buffering** — 20 samples batched per send — because per-sample HTTP couldn't keep up.

`2.3.7/relay.js` is the first PC-side Node process: a UDP↔WebSocket relay. **This is the structural turning point** — the browser stops talking to the Arduino directly, and a PC-side server becomes the middleman for good.

### Phase 3 — The "DS" data model on WiFi UDP (Jan 10–19)
`DBinOut`, `DBinOut_1plus`, `DSlean`, `DSlean2`, `DSV_2.0.0`, `DSV_3.0.0`, `udp-gui-test`

HTTP is abandoned for raw UDP. The key invention here is the **DS data model**: fixed-size typed arrays, addressed by index, mirrored on both sides.

```
dsBools[32]    dsDoubles[32]   dsStrings[8][64]    // telemetry: board → PC
ctrlBools[32]  ctrlDoubles[32] ctrlStrings[8][64]  // control:   PC → board
```

with a deliberately tiny user-facing API: `dsWriteBool(i, v)`, `dsRead(i)`, `dsWriteString(i, s)`.

`DSV_3.0.0` is the high-water mark of this phase — a `TelemetryConfig` struct, configurable refresh rate, CSV/JSON logging options, and the health enum. It's also where the sketch gets big enough that you clearly started wanting it to be a *library* instead of a sketch.

### Phase 4 — Off WiFi, onto Ethernet (Jan 23)
`EnetStack_V1.0.0`

WiFi is dropped for a wired Ethernet shield. Telemetry becomes **one binary `TelemetryFrame` struct memcpy'd into a single UDP packet** — no JSON, no parsing, deterministic size.

The comment `// Public API (Library Functions)` appears over `telemBool()`, `telemInt()`, `telemStr()`, `healthBool()`... You're writing library code inside a sketch, about to extract it.

The reason for the switch is implied rather than stated: determinism. WiFi reconnect handling consumed a lot of `DSV_3.0.0`, and a control link that drops is not a control link.

### Phase 5 — RemoteIO, the actual library (Jan 24–Feb 1)
`ZZZ Versions/remoteIO Archived 0.0.0/` (`RemoteIO.h` + `.cpp`), `RIO`, `RIO20260130`, `giga-gui/`, `ZZZ Versions/RemoteIO Stack/Version 0.0.1/`

The extraction happens. `RemoteIO.h` describes itself as:

> *"RemoteIO — Minimal, deterministic telemetry/control library"*

Capacity grows to **128 bools, 64 ints, 32 doubles, 12 strings**, each way, sent as **4 packets per frame** (packet 0 = bools + timestamp, 1 = ints, 2 = doubles, 3 = strings). Extras appear that only matter if you're serious: pluggable time sources (`setTimeSourceSeconds`), bulk read-only accessors *explicitly for SD logging*, and a control timestamp so you can age-out stale commands.

`RIO.ino` adds a `TelemetryStatus` struct (`connection_ok`, `rtc_ok`, `ethernet_ok`, `sd_logging_ok`, `node_id`, `cpu_temp_c`, `battery_mv`, `fw_version_str`) and a `ControlStatus` with `syncTime` — operator-initiated RTC sync from the console. **`node_id` implies you were thinking about more than one board on the network.**

The PC side becomes a four-process stack (see below), and this is where a working screenshot exists: `RIO ui.png`, dated 2/1/2026, "WS: Connected", telemetry and control columns live.

### Phase 6 — Back to named keys, over HTTP/JSON (Apr 15–19)
`JSONgui`, `TOE2.0_20260415_1931`

A two-and-a-half-month gap, then a restart with a different philosophy.

`JSONgui.ino` throws out the binary frames and the index-addressed arrays and goes back to a plain HTTP server emitting `{"a0":123,"tempC":21.5,"unixtime":...,"duty_pct":4.2}` — **named keys, self-describing**. Its UI is one line of raw JSON across the top and a **"Load Custom UI" button that loads any HTML file you pick into an iframe**.

That button is a significant idea: stop hand-building the dashboard, let the board expose named data and let the operator bring their own front-end.

`TOE2.0` then re-wraps that in the clean `telem.write("a0", ...)` library API shown above. So the final design is: **the ergonomics of Phase 6's named keys, carrying the health/time/control feature set of Phase 5.**

---

## The PC-side stack (Phase 5, `giga-gui/`)

Four cooperating processes, which is where the complexity got away from you:

| Piece | Role | Talks to |
|---|---|---|
| `server.js` | runtime router; builds `controlInt[]` from mapped joystick data; hot-reloads `config.json` | browser (ws :8080), joystick-service (ws :9002), Arduino (UDP 5005/5006) |
| `joystick-service.js` | HID polling at 25 Hz, multi-controller, identifies devices as `vid-pid-serial` | `mapping.json`, server.js, joySetup.html |
| `index.html` | "RemoteIO Diagnostic UI" — telemetry left, controls right, status frame | server.js |
| `joySetup.html` | joystick calibration & mapping editor | joystick-service.js |
| `config.json` | which joystick axis/button lands in which `controlInt[]` index | — |
| `mapping.json` | per-controller HID byte→axis/button decode, min/max/trim/expo | — |

`joySetup.html` (screenshot `Joysetup.png`) is genuinely clever: it shows **all 64 raw HID bytes live as numbers plus per-bit LEDs**, and you point at a byte and declare "this is Axis 0, min -100, max 100, trim 0, expo 0.5" or "this is a bitmask of buttons". It's a reverse-engineering tool for *any* unknown controller, not a driver for one specific pad. The detected device in the saved mapping is a Sony DualShock 4 (`054c-09cc`).

---

## Where it actually stopped, in your own words

`ZZZ Versions/RemoteIO Stack/Version 0.0.1/read me.txt` is the only prose in the archive and it's the real status report:

> *"Arduino was talking to server.js reading and writing control and telemetry to index.html but in the course of trying to get the joystick pulled in it is now bricked"*
>
> *"joysetup.html is doing a lot of heavy lifting at the moment to handle parsing and mapping. Heavy lifting that'll need to be duplicated in joystick service. **Kinda regretting going this route.**"*

So: **a working telemetry+control loop was sacrificed to joystick integration, and the mapping logic ended up in the browser where it couldn't be reused by the headless service.** You'd already diagnosed it. The stated intent behind that design was sound — `joySetup.html` writes `mapping.json` so the user can close the browser and *not* route control packets through a web page, and can hand-tune mappings in the file afterward.

The Phase 6 restart in April looks like the response to that: abandon the four-process stack, go back to something simple enough to hold in your head.

---

## The tension that drove every rewrite

One design question recurs in every phase and never gets settled:

**Fixed index-addressed arrays, or named keys?**

- *Indices* (`dsDoubles[7]`, `telemInt(0, v)`) → fixed packet size, no parsing, deterministic, fast. But both ends must agree on what index 7 means, and nothing enforces it. Every GUI is hand-built to match one sketch.
- *Named keys* (`telem.write("a0", v)`) → self-describing, GUI can be generic, nothing to keep in sync. But variable-size packets, JSON parsing, and non-deterministic timing.

Phases 3–5 go all-in on indices. Phase 6 reverses it. `TOE2.0`'s `telem.max_val(128)` is an attempt to have both — named keys on top, a bounded fixed-capacity store underneath.

**This is the open design decision the project ended on.**

---

## Hardware involved

- **Arduino GIGA R1 WiFi** — the main board throughout
- Ethernet shield (Phase 4 onward); board at `192.168.0.50`/`.51`, PC console at `192.168.0.175` / `.8`, UDP 5005 (board listens) / 5006 (PC listens)
- **DS3231 RTC** over I2C — timestamps and time sync
- Sony DualShock 4 controller over USB HID on the PC
- A servo on pin D2 (`DSlean2`, `SoloJoy`) — the first real actuator driven by joystick input
- External RGB LED header, analog in on A0
- `DSV_2.0.0` uses `WiFiS3`, suggesting an UNO R4 WiFi was tried too

---

## Things in here worth keeping

1. **The `TOE2.0` API sketch.** It's a finished spec for an unfinished library. Build to it.
2. **`joySetup.html`'s raw-HID-byte mapper.** Controller-agnostic, reusable, and the hardest part to re-derive.
3. **The health/status frame.** `TelemetryStatus` + `ControlStatus` is a thought-through set of fields.
4. **The "Load Custom UI" iframe idea.** Decouples the board from its dashboard.
5. **The scaled-telemetry GUI** (`well developed gui.txt`): per-channel raw value, slope, offset, and scaled value — calibration in the UI, not in the firmware.

## Things that were the trap

1. Four processes with hand-edited JSON between them, for a one-board project.
2. Mapping/parsing logic in a browser page that a headless service also needed.
3. Hand-building a new GUI for every change to the index layout.
4. Hardcoded WiFi credentials in twelve sketches.

---

## Housekeeping note

The twelve sketches that contained your WiFi password in plaintext have had it replaced with `REDACTED_WIFI_PASSWORD` in `~/projects/toe/archive/`. The network name is not in the archive either. The original untouched zip is still in `~/inbox/TOE archive.zip`.

`node_modules/` was not copied in — it was ~4.5 MB of the 5 MB archive and is reinstallable from `package.json`.
