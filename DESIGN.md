# MicroRIO — Design

*Driver station in Rust, firmware library in Arduino C++.
Revision 2, 2026-10-24, incorporating the operator's review. Nothing built yet.*

Successor to the 2026 TOE project — see `MicroRIO-origins.md`.
Hardware survey and measured memory budget in `HARDWARE.md`.

---

## Scope

| Piece | Language | Runs on |
|---|---|---|
| `microrio-proto` | Rust (`no_std`-capable) | shared codec, no I/O |
| `microrio-station` | Rust + egui | Linux and Windows desktop |
| `MicroRIO` | Arduino C++ | AVR (Uno, Mega) and ESP32 |

No web server anywhere, on the board or the host. The board does UDP in and UDP out.
Flashing is over USB, always. No simulator — **all testing is against real hardware.**

---

## 1. Direction is the thing people get wrong

This caused confusion in review and it deserves to be the first section.

Every channel is declared with an explicit direction, and that direction determines
**both** the firmware call and the GUI widget. They are mirror images.

| Direction | Firmware declares | Firmware uses | Station does | GUI widget |
|---|---|---|---|---|
| **Telemetry**<br>controller → station | `rio.telemetry_f32("battery_v")` | `rio.write("battery_v", v)` | receives, logs, displays | read-only value, plot, LED |
| **Control**<br>station → controller | `rio.control_f32("drive_x")` | `rio.read_f32("drive_x")` | **populates**, sends, logs | editable field, slider, button, joystick target |

The thing to hold onto: **`read` on the controller means "read what the station sent me."**
Those values do not come from the board's own hardware — they have to be *populated in the
GUI*, by a slider, a checkbox, or a joystick mapping. A control channel with nothing driving
it in the GUI sits at its declared default forever.

The old TOE API hid this badly: a single object called `telem` had a `readBool()` on it that
actually returned control data. `MicroRIO` names the object for the link rather than the
direction, and puts the direction in the declaration.

---

## 2. Settled decisions

| Question | Answer |
|---|---|
| Name | **MicroRIO** |
| Node count | Several simultaneously; `node_id` in every frame |
| Discovery | Manual IP entry, saved station profiles |
| Log format | Native binary at full rate, CSV export on demand. Telemetry **and** control |
| Rate | **One single rate per node.** No rate groups |
| Naming | Named keys negotiated once via schema hash, numeric slots on the wire |
| Floats | f32 |
| Bools | Bitpacked |
| One-shots | Monotonic counters, never pulses |
| Safety | **Enable per node, estop global**; **UI loss is a fault** |
| Capacity | Fixed at compile time in the sketch |
| GUI | egui / eframe |
| Joystick | `gilrs`, with a live validation panel |
| Config | TOML profiles |
| Testing | Real hardware. Mega primary, Uno in the compile matrix, ESP32 second |
| Record keeping | Telemetry records are numbered; the station acks, the node resends |

---

## 3. Single rate

**Changed in review.** Earlier revision split channels into fast / slow / on-change groups.
That is gone. Every channel on a node is sampled and sent at that node's one rate.

The reason is log quality, and it's a good one: a single rate makes the log **perfectly
rectangular**. Every channel has a real sample at every tick. No forward-fill, no "last known
value" ambiguity, no interpolation when you line two channels up six months later. CSV export
becomes a straight dump rather than a resampling problem.

The cost is bandwidth spent re-sending values that didn't change — mostly strings. It turns
out not to matter. Worst case measured from `HARDWARE.md`'s large config:

```
64 bools    8 B        header     12 B
32 i32    128 B        -----------------
32 f32    128 B        total     436 B/frame
 4 str    128 B
16 cmd     32 B        50 Hz  -> 21.8 KB/s each direction
                      100 Hz  -> 43.6 KB/s each direction
```

Comfortably inside 10/100 Ethernet, and **436 bytes is well under the ~1400-byte MTU limit**,
so a full-capacity node still sends one unfragmented datagram per tick. Single rate costs
nothing we can't afford.

Rate is per node, set at runtime, default 50 Hz. A sensor board at 10 Hz alongside a drive
board at 100 Hz is still fine — what's gone is *different rates for different channels on the
same board*.

Keep string channels few. They're the only real waste.

---

## 4. Safety — UI loss is a fault

**Changed in review.** Revision 1 said the UI was a pure subscriber and control continued
without it. That was wrong for a driver station, and the operator said so.

**Three independent paths into fault, all ending in neutral outputs:**

| Trigger | Detected by | Result |
|---|---|---|
| UI stops rendering / closes / hangs | station net thread, via UI heartbeat | station enters FAULT, broadcasts FAULT to every node |
| Control frames stop arriving | controller watchdog (default 200 ms) | controller falls back to its failstates on its own |
| Operator hits estop | station | FAULT to every node |
| A node stops reporting | station, per-node watchdog | alarm in that node's header; nothing automatic |

The UI heartbeat is a counter the render loop bumps. The network thread checks it; stale
beyond threshold (default 250 ms) means the operator can no longer see or intervene, so the
machine must stop.

**The station keeps transmitting while in fault.** Going silent would also fault the
controllers via watchdog, but explicitly sending FAULT is better: the controller knows *why*
it stopped, and telemetry keeps flowing so you can diagnose it. Silence is the backstop, not
the mechanism.

This is precisely why the UI and the network loop stay on separate threads. The separation was
never about running the machine without an operator — it's so that **a hung UI cannot prevent
the FAULT frame from being sent.** If the GUI could block the network thread, the most
important packet in the system would be the one that never goes out.

**Estop is global; enable is per node.** One board can be driving while another is parked,
because parking one board is routine and stopping everything is an emergency. Enable is
runtime state and is never loaded from a profile: opening someone else's file must not be
able to decide what is live.

**Failstates.** A control channel may declare what it reads as while the node's watchdog is
tripped — `rio.failstate(c_led, false)` — so every safe value for a board sits in one place
in its sketch. A channel with no failstate keeps the last value it was sent; nothing is
decided on the sketch's behalf.

**Two watchdogs, one in each direction.** Station to node is the one above, and it is the
node's own business what it does about it. Node to station is an alarm only: telemetry that
stops for longer than that node's watchdog shows `NO COMMS` in its header. The station does
not act on it, because what should happen when one board of several goes quiet is not a
decision a generic driver station can make.

---

## 5. The schema handshake

The firmware declares named channels at startup. On connect the board sends its schema once;
the station builds a name→slot map; both sides speak in slots thereafter.

```
station                                      node
   |-- SCHEMA_REQ (chunk 0) ------------------->|
   |<-- SCHEMA (hash, info, [(name,type,dir,slot)])   chunked if large
   |                                            |
   |-- CONTROL (hash, seq, t, state, rate, ack, values) ->|
   |<-- TELEM  (hash, seq, echo_t, state, rate, records) -|
```

**The station speaks first, and that matters.** There is no HELLO: the station's opening
packet is a schema request, and a node learns where to send from whatever last spoke to it.
A node never initiates, so the station needs no inbound hole in the host firewall — ufw on
this box is active and the station works through it untouched, because the first packet out
opens the path for the reply.

- `schema_hash` rides in every frame. Changed hash means reflashed board: the station
  re-handshakes automatically. A frame whose hash doesn't match the cached schema is
  **discarded**, never guessed at.
- The GUI is generic — it renders whatever the schema declares, with direction deciding
  whether a channel becomes a display or an input.
- **Schema is chunked.** A board declaring 256 bytes of max frame cannot send 40 channel
  names in one datagram.
- Chunk 0 carries the node's name, rate, max frame size, and how many telemetry records it
  can resend.

### Names live in flash

Channel names sit in **PROGMEM** and stream straight from flash to the socket during the
handshake. They never occupy SRAM. On a 2 KB Uno, holding ~100 names in RAM would eat a fifth
of the board.

---

## 6. Wire protocol

**Version 2.** `spec/protocol.toml` is the single source of truth: the Rust constants and the
C++ header are both generated from it, so neither side can drift. A version mismatch is
rejected loudly rather than negotiated, because two sides guessing at each other is how a
machine ends up driven by a misread frame.

### Header (12 bytes)

```
u16  magic        0x4D52 ("MR")
u8   proto_ver    reject loudly on mismatch
u8   msg_type     1=SCHEMA_REQ 2=SCHEMA 3=CONTROL 4=TELEM
u16  node_id
u32  schema_hash
u16  seq          wrapping; diagnostics only, never gates application
```

Little-endian, no padding, explicit widths.

### Bodies

```
CONTROL  u32 host_time_us | u8 state | u8 flags | u16 rate_hz | u32 ack_record | values
TELEM    u32 echo_host_time_us | u8 state | u8 flags | u16 rate_hz
         u32 newest_record | u8 count | count × (u32 node_time_us | values)
```

- `rate_hz` is how often the node should send telemetry — and so how many rows a second the
  log gets. It never touches the board's own loop, which runs at whatever speed the sketch
  runs at. 0 leaves the sketch's rate in force.
- `ack_record` is the highest record the station holds with nothing missing below it.
- A telemetry frame carries the newest record first, then as many older ones as the station
  has not acknowledged and the node still holds, numbered contiguously downwards.

### Channel types

| Type | Wire | Notes |
|---|---|---|
| `bool` | 1 bit, packed | |
| `i32` | 4 bytes | |
| `f32` | 4 bytes | |
| `str` | 32 bytes fixed | keep the count low |
| `cmd` | 2 bytes | monotonic counter, control direction only |

### A node answers to one station at a time

The original rule was that a node learns its station from the last valid
packet it received. Two stations then split a node between them: each got
about half its telemetry, both sent it commands, and the board obeyed
whichever packet arrived last. It presented as packet loss, which sent the
investigation to the network twice before the real cause was found.

So a node now has an owner:

- The first station to send **control** takes the node.
- Another station's control is **refused** while the owner is live, and the
  refusal is sent back to it carrying `station_conflict` — it has no other way
  to find out, because the telemetry it is waiting for goes to the owner.
  Rate limited to one refusal a second, so being refused cannot itself become
  a flood.
- The owner's telemetry carries the same flag for two seconds after a
  conflict. Being fought over is the operator's business too.
- Ownership lapses when the owner goes quiet for longer than the node's
  watchdog — by which point its outputs are already at their failstates, so
  there is nothing to take away. A station that crashes cannot lock a board
  out.
- **A schema request is always answered**, to whoever asked, without
  disturbing the owner. Looking is allowed; driving is not.

### A second station watches through the first, not through the board

Refusing a second station is right, but a refused station is useless. What a
second operator wants is to see. The first attempt had the board send
telemetry to watchers as well as to its owner, and it was wrong in a way worth
recording, because the mistake is one anybody would make.

**It halved the machine's rate.** Measured on a Mega: the node's worst loop
went from under a millisecond to **2.02 seconds**, and 50 Hz became 4.4 Hz.
Not because of the extra frame — because of *who it was sent to*. A W5100
resolves the destination address before every single packet, and the watcher
was a laptop on WiFi. A radio in power save answers an address request when it
next wakes, a few hundred milliseconds later, and the board blocks until it
does. Each watcher frame cost **377 ms**. The board was asleep more than it
was running, and the machine it was driving stuttered because somebody opened
a window to look at it.

The lesson is not about one chip. It is that **the board must only ever send
to a station that is already talking to it.** Anything else hands a stranger
the power to stall the machine.

So watchers are fed by the station that owns the node:

- The owner forwards each telemetry frame exactly as it came off the wire —
  same header, same body — to the board's own network and to this computer, on
  one UDP port. A watcher decodes them with the same code that decodes a
  direct link, so there is no second path to keep correct.
- The schema goes out with them every two seconds, because a watcher never
  speaks to the board and so can never ask for one.
- A watcher sends **nothing** to the board. Not control, not a schema request,
  not even an answer to a refusal. The board does not know it exists.
- **Except the estop.** A watcher may send that, and the node obeys it: held
  two seconds, refreshed while the button is down, and the owner cannot
  override it. Hearing is free; it is answering that costs.

A computer has an address cache, a real network stack and nothing else to do
between frames, which is why the relay costs the owner nothing measurable and
the board nothing at all. A watcher's own recording is a real recording: 50 Hz,
complete, logged the same way as the owner's.

The station drops to watching by itself when a node turns out to belong to
somebody else, says so in that node's row, and offers to take control — which
is granted once the other station stops talking. `--view` starts a station
that only ever watches.

### Even so, the board protects itself

The relay removes the common case, not the general one. A node still sends to
its owner, and an owner can be a laptop that goes to sleep. So:

- The examples cap the retries — `setRetransmissionTimeout(50)` and
  `setRetransmissionCount(1)` after `Ethernet.begin()` — turning a second and
  a half into a tenth of one.
- Every send to a station that is not the owner is timed, and one costing more
  than 20 ms quiets that path for five seconds. That covers refusals and
  schema replies to strangers.
- If the **owner** turns expensive, the node drops its own rate to 5 Hz rather
  than stalling, so control keeps being read whatever the network is doing.

The conformance suite stalls a send on purpose and checks the node keeps
answering and the cost stays bounded.

### Control is full-state, last-write-wins, never retransmitted

Every CONTROL frame carries complete control state. A dropped frame self-heals on the next
tick. **There is deliberately no reliable-delivery layer for control** — a retransmitted
stale command is worse than a dropped one.

### Telemetry is numbered, and resent until it lands

The record of what happened is the opposite case: a hole in the log is a hole forever, and
nothing is made dangerous by filling it in late. So every telemetry sample carries a record
number, the station echoes back the highest one it holds contiguously, and the node resends
anything newer that it still has.

- **Backfill is RAM, so it is per board.** A node advertises how many records it keeps. A
  Mega or an ESP32 keeps as many as one frame can carry back; a 2 KB board keeps one, which
  is to say none, and streams its frames exactly as before.
- **A resent record keeps its original timestamp**, so it lands back where it belongs in the
  log rather than at the time it arrived. Export sorts by time over a two-second window; the
  CSV stays rectangular.
- **Loss is measured, not estimated.** Records that fall out of the node's history before
  they arrive are counted lost and the ack moves past them, so one hole can never hold every
  later record hostage.
- An Uno therefore gets true loss measurement and no backfill — which is the honest result
  of having no memory to spare, rather than a silent approximation.

Sequence numbers still detect frame-level loss for the health display; they never delay or
reorder application.

### One-shots are counters

```cpp
rio.control_cmd("zero_encoders");
...
if (rio.fired("zero_encoders")) encoder.reset();
```

The station increments a counter; the board fires when the received value differs from the
last it saw. Idempotent and loss-tolerant. A boolean pulse would vanish with one dropped
packet — which is exactly what the old TOE `sync_time` one-shot did.

### Latency without clock sync

The station stamps each CONTROL with `host_time_us` from its monotonic clock; the board echoes
the last value it received back in TELEM. Round-trip falls out with no clock sync, no drift
correction, and no RTC involvement. The RTC is for human-readable log timestamps only.

### Per-node max frame size

Declared by the node during the handshake. An Uno says 256 bytes; an ESP32 says 1400. The
station splits schema and respects the limit. A 1400-byte buffer is nothing on an ESP32 and
17% of total RAM on a Mega.

---

## 7. Firmware API

Transport is injected, so Ethernet shield / WiFi / AP are the same library code with no
conditionals. The library never calls `Ethernet.begin()` or `WiFi.begin()` — you own the
network. There are **no pin APIs in the library at all**, only named variables.

```cpp
#include <Ethernet.h>
#include <MicroRIO.h>

EthernetUDP udp;
// capacity fixed at compile time: bools, i32, f32, str, cmd (per direction)
MicroRIO<16, 8, 8, 0, 4> rio(udp);

void setup() {
    Ethernet.begin(mac, ip);          // your network, your business
    udp.begin(5005);

    rio.node(1, "drivebase");
    rio.station(IPAddress(192,168,0,148), 5006);
    rio.rate(50);                     // Hz, one rate for everything
    rio.watchdog(200);                // ms

    // telemetry: controller -> station, GUI displays these
    rio.telemetry_f32 ("battery_v");
    rio.telemetry_i32 ("enc_left");
    rio.telemetry_bool("limit_top");

    // control: station -> controller, GUI populates these
    rio.control_f32 ("drive_x");
    rio.control_bool("intake");
    rio.control_cmd ("zero_encoders");

    rio.begin();                      // computes schema hash
}

void loop() {
    rio.loop();                       // rx, watchdog, tx when due

    rio.write("battery_v", readBattery());
    rio.write("enc_left",  encLeft.read());

    if (rio.enabled()) {
        drive(rio.read_f32("drive_x"));
    }                                 // else: library already neutralized

    if (rio.fired("zero_encoders")) encLeft.reset();
}
```

### Why it fits a 2 KB Uno

Measured, not estimated — see `HARDWARE.md`.

- **Compile-time capacity.** Pay for declared channels only. A one-size 128-bool library would
  cost ~1.4 KB on an Uno before the sketch does anything.
- **Streaming encode.** The library never holds a frame in SRAM. Values go straight into the
  W5100 socket buffer via `Udp.write()` and parse straight back out. A 256 B TX + 256 B RX
  staging pair would add 512 bytes — a quarter of an Uno's RAM — and roughly triple the
  footprint.
- **Names in PROGMEM.**
- **No `String`, no `malloc`, nothing dynamic.**

Result: 64 bools / 32 i32 / 32 f32 / 4 str / 16 cmd per direction runs in **927 B on an Uno
(45%)**, leaving ~1.1 KB for user code.

---

## 8. Station architecture

```
   gilrs ──► joystick task ──┐
                             ▼
                    control state (in-process)
                             │
         ┌───────────────────┼───────────────────┐
         ▼                   ▼                   ▼
   per-node tx          logger task        watch::Sender
   (one rate)                                    │
         │                                       ▼
         ▼                                   egui UI
   UDP sockets   ◄── UI heartbeat ◄───────────── │
         │            (stale => FAULT)
         ▼
   per-node rx ──► telemetry snapshot ──► watch + logger
```

- **One task pair per node.** Independent rate, schema, watchdog, connection state.
- **`tokio::sync::watch` for snapshots** — last-write-wins, no backpressure. A slow UI sees the
  newest value instead of accumulating a queue.
- **`broadcast`** only for events where history matters (faults, connect/disconnect).
- **UI heartbeat flows back into the network thread.** Stale heartbeat means FAULT.
- **No allocation in rx/tx.** Buffers sized once at handshake and reused.
- Control tick moves to a dedicated OS thread with its own timer if jitter ever matters.
  Designed for, not built day one.

---

## 9. Joystick panel

**Expanded in review.** This is a diagnostic instrument, not just a mapping editor, and it
must work **standalone with no controller connected** so a gamepad can be validated on its own.

Requirements:

1. **Device list** — everything `gilrs` enumerates, with hotplug.
2. **Live view of every axis and button.** Axes as labelled bars with numeric values, buttons
   as LEDs that light on press. Updating at UI rate.
   *This is the troubleshooting tool* — press a button, watch it light, confirm the hardware
   works before blaming the robot.
3. **Naming.** Every axis and button gets a user-assigned label that sticks to that device,
   saved in the profile. "Axis 3" becomes "throttle".
4. **Assignment** to a specific `(node_id, control_key)`. One stick can drive channels on two
   boards.
5. **Per-axis conditioning** — min, max, trim, deadband, expo. Carried forward from the old
   `joySetup.html`, which got this part right.
6. **Live preview of the conditioned value** next to the raw one, so you can see what expo and
   trim are actually doing.

`gilrs` gives named axes and buttons across Linux and Windows with hotplug. The raw-HID byte
mapper from the old archive stays in reserve for devices `gilrs` can't decode — it was the
best thing in the 2026 code and shouldn't be lost, but it's no longer the primary path.

---

## 10. Logging

Both directions, at the node rate, because debugging a control problem without the control
history is guesswork.

**Format:** append-only binary. Schema blocks embedded at session start and whenever a node's
schema changes mid-session. Records are `(station_mono_us, node_id, values)`.

- **One unified log**, all nodes interleaved on the station's timeline.
- Per-node clock offset recorded alongside, from the echo-back RTT.
- Writer owns its own buffer; the network path never blocks on disk.
- Single rate means the log is rectangular — see §3.

**CSV export** is a separate operation: pick a time range and channel subset, get a flat file.

---

## 11. Saved configuration

TOML, human-readable, copyable between machines.

- **Station profile** — node list: name, IP, port, rate, enabled.
  Manual IP entry means boards need stable addresses: static IP in the sketch or a DHCP
  reservation.
- **Channel config** — per node per channel: display name, units, **slope and offset**,
  min/max, plot assignment, colour. Calibration lives in the station, so changing a scale
  factor is a config edit rather than a reflash.
- **Joystick profile** — device → per-axis/button name → `(node_id, control_key)`, with
  min/max/trim/deadband/expo.
- **Plot layouts** — channels per axis, time window, autoscale.

---

## 12. Workspace

```
microrio/
├── spec/protocol.toml        single source of truth
├── microrio-proto/           no_std codec, schema, types - zero I/O, property-tested
├── microrio-station/         egui app: net, joystick, logger, ui
├── arduino/MicroRIO/         C++ library
└── archive/                  the 2026 TOE original, for reference
```

`spec/protocol.toml` generates both the Rust constants and the C++ header. Hand-maintaining
two definitions is how the original drifted out of sync.

No simulator. `microrio-proto` still carries pure codec unit tests — encode/decode roundtrip,
no hardware, no fake board — but every integration test runs against the Mega or the ESP32.

---

## 13. Build order

1. `spec/protocol.toml` + `microrio-proto` codec, roundtrip tests.
2. `arduino/MicroRIO` minimum viable node — schema, telemetry, control, watchdog. On the Mega.
3. `microrio-station` headless core — connect, handshake, control tick, fault logic.
   Verified against the real Mega.
4. Logger + CSV export.
5. egui UI — node panels, live values, plots, enable/estop, UI heartbeat.
6. Joystick panel — live validation view, naming, assignment, conditioning.
7. ESP32 support (`esp32:esp32` core, WiFi and Ethernet transports).
8. Uno in the compile matrix throughout, as the memory canary.

---

## Appendix: what review changed (2026-10-25 → 11-01)

Four rounds of driving the thing and saying what was wrong with it. Where a
decision belongs with the design it has been folded into the body above —
enable per node is in §2 and §4, the record numbering and the rate field are
in §6, the handshake's opening packet is in §5. What is left here is the
reasoning, kept because the reasoning is the part that is hard to recover.

### Enable is per node; estop is global

`enabled` was one flag for the whole station. It is now one per node, held at
runtime and never loaded from a file, with the buttons on each node's own row.
Estop stays global, because an emergency is never about one board, and it
still latches.

A saved profile deciding what is live on startup would be a trap, so the
enable set is runtime-only. `NodeConfig.enabled` — which actually meant "open
a link at all" — was renamed `link_active` to stop the two reading as the same
thing, with a serde alias so old profiles still load.

### A UI stall no longer clears itself

A stalled window faulted every node and then handed them straight back when
rendering resumed. That is wrong: the window drawing again says the window is
drawing, not that anyone is watching. It now disables every node and reports
the stall until it is dismissed.

This was forced by the file dialogs. `rfd` blocks whichever thread opens a
dialog, so a Save box opened from the UI thread tripped the stale-UI fault —
and without latching it would stop the machine and silently restart it.

Latching was the right answer to the fault; opening the dialog on the UI
thread was not. Every dialog now runs on a worker thread and hands its answer
back for the next frame, so picking a profile or exporting a CSV leaves the
machine running and driving. The latch stays for everything else that can
stop the window drawing.

### Control limits are enforced where they are displayed

The control grid's `wire` column showed `clamp(value + offset)` while
`build_values` sent the raw intent. The column was simply wrong whenever an
offset was set. The transform now happens in `build_values`, and the grid and
the frame share one function.

Making the transform real exposed a trap: `ControlConfig::default_for(I32)`
clamped to ±10 000, and `set_unix` carries ~1.8e9. Enforcing the default would
have turned clock sync into sending a timestamp in 1970 — and the probe would
not have caught it, because the probe does not go through the station's
control path. Integer channels therefore default to the full i32 range;
bounds on an integer are something the operator opts into. There is a test
whose only job is to fail if that default ever narrows again.

### Charts name their axes

Arming a channel could only ever produce one shape of chart: that channel
against time. Charts now hold an X axis and a list of Y axes, drawn from the
same list of candidates, where a candidate is elapsed time, clock time, or any
telemetry channel on any node.

Letting time be a legal Y axis is what makes "flip" a plain swap rather than a
special case, and letting a channel be X is what makes an X-Y plot fall out
for free. X-Y is drawn as points rather than a line, because joining samples
in arrival order draws loops that are an artefact of sampling, not of the
data.

Cross-node traces are matched by timestamp with a zero-order hold and a 0.5 s
tolerance: each Y sample takes the most recent X sample at or before it, and
an X older than the tolerance is not a match at all. A dead source leaves a
gap rather than a straight line drawn out of stale data.

Profiles from the previous build are migrated: channels with `plot = true`
become the Y axes of the first chart.

### One zone, and it is never UTC

Every displayed and logged time goes through one zone, set in Settings, either
`system` or an IANA name. The UTC row is gone from the clock panel, because
two clocks on screen in two zones is how you end up reading the wrong one.

`system` is resolved to a real IANA zone via `iana-time-zone`, because
chrono's local zone cannot produce an abbreviation — it formats as `-07:00`
where the named zone formats as `MST`. Converting to `FixedOffset` anywhere in
the path loses the name, so the formatting happens on `DateTime<Tz>`.

The wire is unchanged and still carries plain unix seconds. That is exactly
what makes showing the node's clock in the operator's zone correct rather than
a guess.

### Logging has a start and a stop

The background log still runs for the whole session — nothing is lost by
forgetting to press start, which was the one good property of the old
always-on behaviour. Start and stop now mark a window inside it, and stopping
asks where to put a CSV of just that window.

The log gained a `KIND_META` record holding the wall clock at session start,
so an export can print the time of day a row was taken. Logs without it still
export, minus that column.

Export runs on a worker thread after asking the logging thread to flush, so
the file being read is not missing its last two hundred buffered rows.

### Cross-node links fault rather than hold

A telemetry channel on one node can drive a control channel on another,
scaled and clamped on the way. If the source has not reported for 200 ms the
destination node is commanded to FAULT.

Holding the last value was the alternative and it is the wrong one: a dead
sensor that goes on commanding an output is the exact failure this design
exists to prevent.

### The operator's own instruments

A channel is a number until someone says what it means, and review kept
arriving at the same answer: that saying belongs in the station's profile, not
in the sketch and not on the wire.

- **Value labels.** Any i32 can carry a value→label table. Telemetry shows the
  word, a control becomes a dropdown, and an unmapped value shows its number
  rather than a blank. This is also what "mode" is, which is why there is no
  mode field in the protocol.
- **Groups.** Named sets of channels in a user-set order, mixing telemetry and
  control, collapsible, and pinnable to the node header. A control lives in one
  group so there is one place to set it; telemetry can appear in several.
- **Pinned header items**, capped at five per node so the row still reads at
  half-screen width. Link health, round trip and loss are always there.
- **Thresholds warn and nothing more.** A supply voltage below its limit turns
  orange. What should happen next is the operator's call, or the sketch's.
- **Inverted lamps.** A signal that is normally true — `rtc_ok` — lights when
  it goes false, so on a panel of lamps anything lit means look at me.

None of it reaches the board or the log: the wire carries the integer, the CSV
records the integer.

### Charts the operator cannot get lost in

A plot that can be zoomed and flung by a stray scroll is a plot that can be
left unreadable with no way back, and that is exactly what happened during
review. The chart now computes its own bounds every frame and draws them:
zoom, scroll and boxed zoom are off, dragging slides the window back through
history, and one button returns it to live. Y is either fitted to the data or
typed in, and can be drawn downwards. Fifteen minutes of history is kept and
thinned to min/max per pixel column, so a five-minute window shows every spike
without drawing fifteen thousand points.

### Nothing blocks the window

A native file dialog blocks whichever thread opens it. Opened from the UI
thread it stopped the window drawing, which the station correctly read as a
stall — so picking a profile mid-test disabled every node. The dialogs now run
on worker threads and hand their answer back for the next frame. The stall
fault itself was right and stays.

### Rate: what it is, said honestly

`rate_hz` only ever set the station's send rate. The node's telemetry rate is
compiled into the sketch and advertised in its schema, and nothing on the wire
can change it. Rather than leave a box that looks like it governs everything,
the node row now shows both — the station's tx rate, editable live, and the
node's own rate beside it, turning orange when they disagree.

**Done in v2.** The control frame now carries the rate, and the node adopts
it: one box, one rate, telemetry and control and the log all at it. It governs
how often the node *reports*, never how fast its loop runs — a board can
compute at 1 kHz and report at 50 Hz, and usually should.

The tx rate is now re-read from the profile each tick, so changing it rebuilds
the interval without tearing down the link and re-negotiating the schema.

### Two window sizes are the design target

Full screen and half a screen are the two shapes that get used, so the layout
is built for both: above 1180 px the telemetry and control grids sit side by
side, below it they stack. The chart takes whatever height the grids do not
need, measured from the previous frame's content size rather than a fixed
fraction — a fixed fraction left a band of nothing on a wide screen.

`--size WxH` opens the window at a given size, which is how either shape gets
reproduced without dragging an edge.

One egui trap worth recording: inside a `Grid` cell, `TextEdit::desired_width`
is only a hint and gets ignored, so "counts" rendered as "cou". `add_sized`
is the one that binds.

---

## Appendix: why UDP and not TCP

Measured on the Mega with a W5100 shield, 127-byte frames (the real SimNode
telemetry size), 200 frames per run:

| | per frame | spread |
|---|---|---|
| UDP | **596 us** | none — 596 every run |
| TCP write | 562–987 us | ~75% |
| TCP flush (wait for ACKs) | 1428–4028 us per 25 KB | ~3x |

**TCP is not slower on average.** Its per-write cost is sometimes lower than
UDP's, because a write only copies into the socket buffer rather than framing a
datagram. What it costs is *determinism*: UDP was 596 us on every single run,
TCP varied by three-quarters, plus a flush that varied threefold.

At 50 Hz a send is 3% of the loop budget either way, so throughput is not the
deciding factor. These four things are:

1. **A retransmitted control frame is a stale control frame.** Frames carry
   full state at a fixed rate, so a lost one is superseded 20 ms later. TCP
   would deliver the old one first and hold the newer ones behind it.
2. **TCP buffers on a stall.** If the link hiccups, the board's send buffer
   fills with old frames and replays them in order on recovery — a burst of
   stale commands. UDP discards them, which is the correct behaviour.
3. **TCP notices a dead link slowly.** Retransmit timers and keepalives take
   seconds; the watchdog takes 200 ms. The thing a driver station most needs to
   know is exactly where TCP is weakest.
4. **The reliability we need is already in the protocol, not the transport:**
   Ethernet CRC32 and the UDP checksum mean corrupt frames are dropped rather
   than delivered; sequence gaps measure loss; `cmd` counters make one-shot
   events idempotent and loss-tolerant; the schema hash catches a reflashed
   board.

**Not making it selectable.** A TCP mode would double the firmware surface,
cost SRAM on an Uno (retained send buffers, against the current streaming
encode), and be measurably worse at the one thing that matters most. Where TCP
genuinely wins is bulk transfer and networks you do not control — that belongs
in a separate management channel, added when something needs it, not as a mode
switch on the control loop.

### Headroom

At 596 us per send the W5100 ceiling is ~1670 frames/s if it did nothing else.
Against a 20 ms tick: 3.0% at 50 Hz, 6.0% at 100 Hz, 11.9% at 200 Hz. The
board's own loop measures 928 us including waveform generation and the RTC.
Neither transport is near a limit at the rates in use, so the shield is not the
constraint it looks like.

### Operational note

`ufw` is active on the machine the station runs on. The station works because it speaks first, so
return telemetry matches an established flow. A node that sent telemetry
unsolicited, before any control frame, would be dropped at the host firewall —
which is a good reason the handshake starts from the station.

---

## Open assumptions

Confirmed in review, recorded so they stay visible:

- **Estop is global.** One estop stops every node.
- **32-byte strings**, and few of them.
- **Board-side SD logging deferred.** Bulk accessors stay in the library so it can be added
  later without a protocol change.
- **Raw-HID byte mapper is a reserve path**, behind `gilrs`.
- **IPv4, single subnet.** No routing, no NAT traversal.
