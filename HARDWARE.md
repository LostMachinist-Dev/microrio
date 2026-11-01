# MicroRIO — Hardware on serverbox

*Surveyed 2026-10-24, updated after Darrel confirmed the Ethernet shield and swapped the
ESP32's charge-only cable. Both boards are now present and flashable over USB.*

---

## Arduino Mega 2560 R3

| | |
|---|---|
| Port | `/dev/ttyACM0` @ 115200 |
| USB ID | `2341:0042` Arduino SA Mega 2560 R3 (CDC ACM) |
| Signature | `0x1e9801` — ATmega2560, read via avrdude |
| FQBN | `arduino:avr:mega` |
| Memory | 8 KB SRAM, 254 KB usable flash |
| Network | **Ethernet shield fitted** (confirmed by Darrel) |
| Flash via | USB, `arduino-cli` / `avrdude` |

Current firmware is a bare stub, not MicroRIO: prints `HELLO mega-testbed 1` at boot, answers
`PING` with `OK pong 1`, rejects everything else. It never brings the shield up, which is why
the board had no LAN presence during the first survey.

## Olimex ESP32-POE-ISO-EA-16MB

| | |
|---|---|
| Port | `/dev/ttyUSB0` (CH340, `1a86:7523`) |
| Chip | **ESP32-D0WD-V3 rev 3.1**, dual core, 240 MHz, Wi-Fi + BT |
| Flash | **16 MB** (GigaDevice `c8:4018`) |
| MAC | `e4:65:b8:2a:28:b8` |
| Crystal | 40 MHz |
| Memory | 520 KB SRAM |
| Flash via | USB, `esptool` |

**Flash is blank.** The ROM bootloader (`ets Jul 29 2019`) loops on
`invalid header: 0xffffffff` under the RTC watchdog — erased flash, no app image. That fully
explains its absence from the network in the first survey; there was nothing on it to bring
the PHY up.

The earlier failure to detect it at all was the charge-only USB cable, now replaced.

## Toolchain

| Tool | Status |
|---|---|
| rustc / cargo | 1.99.1 |
| arduino-cli | 1.2.2 |
| `arduino:avr` core | **1.8.8 — installed** |
| `Ethernet` library | **2.0.2 — installed** |
| avrdude | 7.1 |
| esptool | **5.6.0** — venv at `/tmp/claude-1000/-home-yerkovich-projects/b4ec845d-fe81-4d45-84a8-6923194b0f5c/scratchpad/esptool-venv/bin/esptool` |
| `esp32:esp32` core | **not installed** — needed before the Olimex can be built. Large download; say the word. |
| pyserial | 3.5 |

`yerkovich` is in `dialout`; no sudo needed for either board.

## Network

`enp195s0` is up with gigabit carrier but holds only a link-local address
(`169.254.167.135/16`) — no DHCP lease. serverbox's real address is on WiFi
(`192.168.0.148`). Worth sorting before wired nodes go on that port, since the design uses
manual IP entry and assumes station and nodes share a subnet.

---

## Memory budget — measured, not estimated

Compiled a representative MicroRIO node (streaming encode, names in PROGMEM, no frame buffer in
SRAM) at three capacities against the real `arduino:avr` toolchain and `Ethernet` 2.0.2.

Capacity is **per direction** — telemetry and control both.

| Config | bools | i32 | f32 | cmd | str | Uno SRAM | Mega SRAM |
|---|---|---|---|---|---|---|---|
| baseline (Ethernet+UDP only) | — | — | — | — | — | 199 B / 9% | 227 B / 2% |
| small | 16 | 8 | 8 | 8 | 0 | **383 B / 18%** | 411 B / 5% |
| medium | 32 | 16 | 16 | 8 | 2 | **575 B / 28%** | 603 B / 7% |
| large | 64 | 32 | 32 | 16 | 4 | **927 B / 45%** | 955 B / 11% |

Flash is a non-issue: ~8.4 KB total on an Uno (26% of 32 KB), and essentially flat across
configs because capacity costs SRAM, not program space.

**An Uno comfortably runs the large config**, with ~1.1 KB of SRAM left for user code.
That's a better result than expected and it settles the Uno question.

### Why it lands this low: streaming encode

The library **never holds a frame in SRAM.** Values are written straight into the W5100's
socket buffer via `Udp.write()` and parsed straight out of it, field by field.

This is the difference between viable and not. A conventional 256-byte TX plus 256-byte RX
staging buffer would add 512 bytes — **25% of an Uno's entire RAM** — and nearly triple the
library's footprint. Socket buffers live on the W5100 chip, not on the MCU, so streaming gets
them for free.

The same applies to the schema handshake: channel names sit in **PROGMEM** and stream from
flash directly to the socket. They never occupy SRAM at any point.

---

## Revisions to DESIGN.md

### 1. Serial transport drops from required to optional

The first survey concluded serial had to be first-class because the Mega appeared to have no
network. With the shield confirmed, that's wrong — **UDP works on every board in play.**

Keep the transport abstraction so a serial backend *can* be added (a bare Uno with no shield
as a USB-attached node is genuinely useful), but build UDP first and don't hold the project up
for it.

### 2. Compile-time capacity is mandatory

The table above only works because capacity is fixed at compile time. A one-size library with
128 bools and 64 ints would cost ~1.4 KB on an Uno before the sketch does anything.

Declare capacity in the sketch and pay for exactly what's declared.

### 3. Names in PROGMEM — confirmed and still required

Unchanged from the first survey, and the measurements back it up.

### 4. Per-node negotiated max frame size — confirmed

An Uno should declare something like 256 bytes. The station splits rate groups to fit whatever
each node advertises.

### 5. Put the Uno in the compile matrix as the canary

Develop against the Mega as agreed, but **compile for `arduino:avr:uno` on every build.** It
has a quarter of the Mega's RAM, so it catches memory regressions the moment they appear
rather than months later when someone tries to deploy on one. Costs seconds per build.

### 6. Flashing is USB for both boards, always

Per Darrel. No OTA, no network bootloader in scope. `arduino-cli` for AVR, `esptool` for the
ESP32.
