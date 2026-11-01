#!/usr/bin/env python3
"""Build the MicroRIO study guide.

    python3 docs/build-study-guide.py

Writes ~/deliverables/microrio/microrio-study-guide.docx, overwriting it.
Content lives in this file so the guide can be regenerated whenever the
project moves on: edit here, re-run, download.
"""

import pathlib
import subprocess

from docx import Document
from docx.enum.text import WD_ALIGN_PARAGRAPH
from docx.shared import Pt, RGBColor

OUT = pathlib.Path.home() / "deliverables" / "microrio" / "microrio-study-guide.docx"
ROOT = pathlib.Path(__file__).resolve().parent.parent


def git(*args, default="unknown"):
    try:
        return subprocess.run(
            ["git", "-C", str(ROOT), *args],
            capture_output=True, text=True, check=True,
        ).stdout.strip() or default
    except Exception:
        return default


doc = Document()

# A monospace style for commands, and a lighter one for the "where it runs" tag.
styles = doc.styles
mono = styles.add_style("Command", 1)
mono.base_style = styles["Normal"]
mono.font.name = "DejaVu Sans Mono"
mono.font.size = Pt(9.5)
mono.paragraph_format.space_before = Pt(2)
mono.paragraph_format.space_after = Pt(6)
mono.paragraph_format.left_indent = Pt(18)

where = styles.add_style("Where", 1)
where.base_style = styles["Normal"]
where.font.size = Pt(9)
where.font.bold = True
where.font.color.rgb = RGBColor(0x55, 0x55, 0x55)
where.paragraph_format.space_after = Pt(0)
where.paragraph_format.left_indent = Pt(18)


def h(text, level=1):
    doc.add_heading(text, level)


def p(text=""):
    doc.add_paragraph(text)


def bullets(items):
    for it in items:
        doc.add_paragraph(it, style="List Bullet")


def cmd(command, run_in="box", note=None):
    doc.add_paragraph(f"Run in: {run_in}", style="Where")
    doc.add_paragraph(command, style="Command")
    if note:
        doc.add_paragraph(note)


def table(headers, rows):
    t = doc.add_table(rows=1, cols=len(headers))
    t.style = "Table Grid"
    for cell, text in zip(t.rows[0].cells, headers):
        cell.text = text
        for par in cell.paragraphs:
            for run in par.runs:
                run.bold = True
    for row in rows:
        cells = t.add_row().cells
        for cell, text in zip(cells, row):
            cell.text = str(text)
    doc.add_paragraph()


def notes(lines=5):
    doc.add_paragraph("My notes").runs[0].bold = True
    for _ in range(lines):
        par = doc.add_paragraph("_" * 95)
        par.paragraph_format.space_after = Pt(4)
    doc.add_paragraph()


# --------------------------------------------------------------- title

title = doc.add_heading("MicroRIO — Study Guide", 0)
title.alignment = WD_ALIGN_PARAGRAPH.LEFT
p(f"Build {git('describe', '--always', '--dirty=+')} · "
  f"{git('log', '-1', '--date=short', '--format=%cd')} · "
  f"{git('rev-list', '--count', 'HEAD', default='?')} commits")
p("A driver station for microcontrollers, and the Arduino library that talks "
  "to it. This guide is the plain-English version: what it is, how it was "
  "built, and how to pick it up again months from now.")

# ------------------------------------------------------- 1. big picture

h("1. The big picture")
p("MicroRIO is two pieces of software that talk to each other over an "
  "ordinary network:")
bullets([
    "The driver station — a desktop app that runs on your computer. It shows "
    "live readings, lets you set values, draws charts, and records everything.",
    "The Arduino library — code that runs on a microcontroller board. You "
    "name your signals in the sketch; the library handles the network.",
])
p("The idea it is built around: name your channels once, then stop sending "
  "names. When the station connects, the board sends a list of its channels "
  "(the 'schema'). After that both sides refer to a channel by its number in "
  "that list. A reading is four bytes on the wire, not the word 'battery_v' "
  "plus four bytes. That is what lets it run on a board with 2 KB of memory.")
p("Each channel has a direction, and this is the thing people get wrong:")
table(["Direction", "Who writes it", "What the station does"], [
    ["Telemetry (board -> station)", "Your sketch, with write()", "Displays it"],
    ["Control (station -> board)", "You, in the app", "Sends it; your sketch reads it"],
])
p("A control channel with nothing driving it sits where you left it. That is "
  "not a fault; nothing on the board fills it in.")
p("How the pieces connect:")
cmd("sketch (MicroRIO.h)  <--UDP-->  station (microrio-station)\n"
    "        |                               |\n"
    "   your hardware                 charts, logs, CSV",
    run_in="concept, not a command")
p("Both sides are generated from one file, spec/protocol.toml. It describes "
  "the wire format; a build step turns it into Rust constants and into a C++ "
  "header. Neither side can drift from the other, because neither is written "
  "by hand.")
notes()

# ---------------------------------------------------------- 2. glossary

h("2. Glossary")
table(["Term", "What it means"], [
    ["Node", "One board. A station can talk to several at once."],
    ["Channel", "One named value, in one direction. For example battery_v."],
    ["Schema", "The list of channels a board declares. Sent once at connect."],
    ["Schema hash", "A number computed from that list. If it changes, the board "
                    "was reflashed and the station starts again."],
    ["Slot", "A channel's position in the schema. What actually goes on the wire."],
    ["Telemetry", "Readings from the board to the station."],
    ["Control", "Values from the station to the board."],
    ["Frame", "One network packet of values."],
    ["Record", "One numbered telemetry sample. Numbering is what lets a lost "
               "one be resent."],
    ["Backfill", "The board resending records the station never received."],
    ["Ack", "The station telling the board the highest record it holds."],
    ["Watchdog", "A timer that expects regular contact. If it runs out, "
                 "something has gone wrong and the side that noticed says so."],
    ["Failstate", "What a control reads as on the board while its watchdog is "
                  "tripped. Set in the sketch with rio.failstate()."],
    ["E-stop", "Emergency stop. Latches; disables every node until cleared."],
    ["Enable", "Per node. Whether that board's outputs may be live."],
    ["Profile", "A .toml file holding your nodes, channel settings, charts and "
                "joystick mappings."],
    ["Cross-node link", "One board's reading driving another board's control."],
    ["Owner", "The one station a node accepts control from."],
    ["Watcher", "Any other station. It sees everything at full rate through "
                "the station that is driving, and can set nothing — except "
                "the estop, which always works."],
    ["Relay", "How a watcher is fed: the driving station forwards what it "
              "receives. The board never sends to a watcher."],
    ["Pass-through", "What those links are called on the Operator page."],
    ["UDP", "A way of sending packets with no delivery guarantee and no "
            "handshake. Fast and predictable, which is what control needs."],
    ["TCP", "The reliable alternative. Rejected here: it retries, and a "
            "retried command arrives stale."],
    ["PROGMEM", "Arduino's name for flash memory. Channel names live there, "
                "not in the board's small RAM."],
    ["Rust", "The language the station is written in."],
    ["Cargo", "Rust's build tool. 'cargo build', 'cargo test'."],
    ["Crate", "A Rust package. This project has four."],
    ["egui", "The toolkit the station's window is drawn with."],
    ["gilrs", "The library that reads joysticks and game controllers."],
    ["xtask", "A convention for project tools run as 'cargo xtask <thing>'."],
    ["arduino-cli", "Command-line Arduino. Compiles and flashes without the IDE."],
    ["NSIS", "The tool that builds the Windows installer."],
    ["deb", "A Linux installer file, for Debian and Ubuntu."],
    ["Xvfb", "A fake screen, so the app can be run and photographed with no "
             "monitor attached."],
    ["wine", "Runs Windows programs on Linux. Used to check the .exe opens."],
])
notes()

# ------------------------------------------------------ 3. what we did

h("3. What we did, step by step")

h("Step 1 — Work out what the old project was", 2)
p("The project started from a zip of earlier work (TOE). It was read, copied "
  "into archive/, and a hardcoded WiFi password was removed from it. The "
  "intent was written up as MicroRIO-origins.md, and the plan as DESIGN.md.")

h("Step 2 — Measure the hardware before designing for it", 2)
p("Rather than assume what fits on a small board, the real toolchain was used "
  "to measure. An Uno runs the large configuration in 927 bytes, leaving room "
  "for a sketch. That settled the design: compile-time capacity, names in "
  "flash, frames streamed straight to the socket.")
cmd("arduino-cli compile -b arduino:avr:uno --library arduino/MicroRIO "
    "arduino/MicroRIO/examples/BasicNode")

h("Step 3 — One source of truth for the protocol", 2)
p("spec/protocol.toml describes the wire format. A build step generates the "
  "Rust constants, and a command generates the C++ header.")
cmd("cargo xtask gen-header")

h("Step 4 — The codec, with tests that try to break it", 2)
p("microrio-proto encodes and decodes frames. Its tests use proptest, which "
  "generates thousands of random cases rather than the few a person thinks "
  "of: random channel layouts, NaN, oversized strings, every truncation of "
  "every frame.")
cmd("cargo test -p microrio-proto")

h("Step 5 — Prove the two languages agree, on real boards", 2)
p("The classic cross-language bugs — byte order, bit packing, padding — do "
  "not show up in one language's tests. The Conformance sketch runs the "
  "protocol over USB instead of the network, so the C++ encoder can be "
  "checked against the Rust one with no network in the way. 38 checks, on "
  "both an AVR and a 32-bit ESP32.")
cmd("arduino-cli compile -b arduino:avr:mega --library arduino/MicroRIO -u "
    "-p /dev/ttyACM0 arduino/MicroRIO/examples/Conformance\n"
    "cargo xtask conformance /dev/ttyACM0")

h("Step 6 — The station", 2)
p("A desktop app: node list, telemetry and control grids, charts, joystick "
  "panel, logging, settings. The network loops are kept off the drawing "
  "thread on purpose, so a frozen window cannot stop the FAULT frame going "
  "out — the one packet that must never be the one that fails to send.")
cmd("cargo run -p microrio-station")

h("Step 7 — Check the live link, not just the code", 2)
p("The probe drives a real board over the network and checks what comes back: "
  "rate, round trip, packet loss, the watchdog, the clock, and backfill. It "
  "drops every fourth frame on purpose to make the board resend.")
cmd("cargo xtask probe 192.168.0.50 1")

h("Step 8 — Protocol version 2, after review", 2)
p("Reviewing the design turned up three things to bundle into one version "
  "bump and one reflash: numbered telemetry records with resending, the "
  "station setting the telemetry rate, and the doc fixes that went with them.")

h("Step 9 — Four rounds of using it and fixing what was wrong", 2)
p("Each round was: run the real app against real boards, say what was wrong, "
  "fix it. That is where the big e-stop, the groups, the value labels, the "
  "chart that cannot get lost, the typed number fields, the per-node clock "
  "page and the non-blocking file dialogs came from.")

h("Step 10 — Package it for other machines", 2)
p("One script builds everything into the downloads folder.")
cmd("packaging/release.sh")
notes()

# -------------------------------------------------- 4. where things are

h("4. Where everything lives")
table(["Path", "What it is"], [
    ["~/projects/microrio/", "The project"],
    ["  spec/protocol.toml", "The wire format. The single source of truth"],
    ["  microrio-proto/", "The codec, shared by the station and the tools"],
    ["  microrio-station/", "The driver station app"],
    ["  arduino/MicroRIO/", "The Arduino library"],
    ["  arduino/MicroRIO/examples/", "BasicNode, SimNode, Esp32PoeNode, "
                                     "Esp32WifiNode, Conformance"],
    ["  arduino/flash-wifi-node.sh", "Flashes the WiFi node without putting "
                                     "the password in the project"],
    ["  xtask/", "gen-header, conformance, probe"],
    ["  packaging/", "release.sh, the NSIS installer script, icon, .desktop"],
    ["  docs/build-study-guide.py", "Rebuilds this guide"],
    ["  DESIGN.md / RUNNING.md / TASKS.md", "Why it is built this way / the "
                                            "operator's manual / the task list"],
    ["~/deliverables/microrio/", "Everything to download"],
    ["~/.config/microrio/station.toml", "Your live profile"],
    ["~/.config/microrio/wifi.txt", "WiFi name and password. Never in the project"],
    ["~/.local/share/microrio/logs/", "Session logs (.mrlog)"],
    ["~/.local/bin/microrio-station", "The installed Linux app"],
    ["/dev/ttyACM0", "The Mega, over USB"],
    ["/dev/ttyUSB0", "The ESP32, over USB"],
    ["192.168.0.50", "The Mega, on the network"],
    ["192.168.0.51", "The ESP32, on WiFi"],
])
notes()

# ------------------------------------------------------- 5. cheat sheet

h("5. Command cheat sheet")
p("Everything here runs on the box, in ~/projects/microrio, unless it says "
  "otherwise.")

h("Build and test", 2)
cmd("cargo test --workspace", note="All the tests. 91 of them.")
cmd("cargo run -p microrio-station", note="Run the station from source.")
cmd("cargo build --release -p microrio-station", note="Build the fast version.")
cmd("packaging/release.sh", note="Build everything into the downloads folder.")

h("The boards", 2)
cmd("arduino-cli board list", note="What is plugged in.")
cmd("arduino-cli compile -b arduino:avr:mega --library arduino/MicroRIO -u "
    "-p /dev/ttyACM0 arduino/MicroRIO/examples/SimNode",
    note="Compile and flash the Mega with the simulator sketch.")
cmd("arduino/flash-wifi-node.sh /dev/ttyUSB0",
    note="Flash the ESP32 for WiFi, taking the password from "
         "~/.config/microrio/wifi.txt.")
cmd("cargo xtask conformance /dev/ttyACM0",
    note="38 protocol checks over USB. Needs the Conformance sketch flashed.")
cmd("cargo xtask probe 192.168.0.50 1",
    note="Check the live link to node 1. Stop the station first; two stations "
         "fight over a board.")

h("Logs", 2)
cmd("ls -t ~/.local/share/microrio/logs/ | head", note="The most recent sessions.")
cmd("microrio-station export ~/.local/share/microrio/logs/NAME.mrlog ~/csv",
    note="Turn a log into CSV without opening the window.")

h("The app", 2)
cmd("microrio-station", note="Start it.")
cmd("microrio-station --version", note="Which build this is.")
cmd("microrio-station --tab chart", note="Open straight to the charts.")
cmd("microrio-station --size 960x1080", note="Open at a given size.")
cmd("microrio-station --profile ~/some/other.toml", note="Use a different profile.")
notes()

# --------------------------------------------------------------- 6. FAQ

h("6. FAQ")

h("How do I start work on this again?", 2)
cmd("cd ~/projects/microrio && cat TASKS.md")
p("TASKS.md says what is done, what is next and what is blocked. git log "
  "--oneline shows the history.")

h("How do I check it still works?", 2)
cmd("cargo test --workspace\ncargo xtask probe 192.168.0.50 1")
p("Tests first, then a real board. Stop the station before probing.")

h("How do I download the builds?", 2)
p("Open http://192.168.0.148:8000/deliverables in a browser on the laptop.")

h("I changed the protocol. What do I have to rebuild?", 2)
cmd("cargo xtask gen-header\ncargo test --workspace")
p("Then reflash every board: both sides must speak the same version, and a "
  "mismatch is rejected rather than guessed at.")

h("The station shows a node but no values.", 2)
p("It is waiting for the schema. If it never arrives: check the IP in "
  "Settings, check the board is powered, and check nothing else is talking "
  "to it. A node follows whoever spoke to it last, so a second station "
  "steals it.")

h("The readings are jumping or the loss figure is high.", 2)
p("Usually two stations on one board. Close the other one. The header's loss "
  "figure counts records that never arrived after resending, so it is real "
  "loss, not dropped packets that were recovered.")

h("A watcher shows nothing, or no round trip.", 2)
p("A watcher is fed by the station that is driving, not by the board. No "
  "round trip is correct — it never speaks to the board. If it is blank, the "
  "driving station is closed, has relaying turned off in Settings, or is on a "
  "different network.")

h("A board says VIEW ONLY.", 2)
p("Another station is driving it. You see everything it reports, at full "
  "rate, and can chart and record it — you just cannot set anything. The "
  "E-stop still works, and the station driving the board cannot override it. "
  "Press take control to ask for the board; you get it once the other "
  "station stops talking to it. Start a station with --view if you only ever "
  "want it to watch. The probe counts as a station too, so stop the app "
  "before probing.")

h("I want a channel to show a word instead of a number.", 2)
p("Settings -> Channel display -> Labels. Give each value a name. A telemetry "
  "i32 then shows the word; a control i32 becomes a dropdown. The log still "
  "records the number.")

h("A lamp is lit when everything is fine.", 2)
p("Settings -> Channel display -> Invert. Then the lamp lights when the "
  "signal reads false, which is what you want for something like rtc_ok.")

h("How do I record a run?", 2)
p("Logging tab -> Start. Everything is already being recorded in the "
  "background; Start marks the piece you care about. Stop asks where to save "
  "it as CSV.")

h("How do I add a board?", 2)
p("Settings -> Nodes -> Add node, set its id, label and IP, then Apply. The "
  "id must match rio.node(id, ...) in the sketch.")
notes()

# ------------------------------------------------------ 7. what went wrong

h("7. What went wrong and how it was fixed")
table(["Symptom", "Cause", "Fix"], [
    ["The wire column showed one number, the board got another",
     "The offset was applied where it was displayed, not where the frame was "
     "built",
     "One function that both use. A test now compares them"],
    ["battery_v read a sensible-looking voltage from nothing",
     "An unconnected pin settles near its neighbour, so it looked wired",
     "The sketch simulates the pack voltage and says so; a0 stays a real read"],
    ["Readings kept dropping out; the network got blamed twice",
     "Two copies of the station talking to one board. The board followed "
     "whoever spoke last, so each station got about half the telemetry and "
     "both sent commands",
     "A node accepts control from one station; any other becomes a watcher "
     "that sees everything and sets nothing. The app refuses to start twice "
     "on one computer"],
    ["Clock sync sent the year 1970",
     "Integer controls defaulted to a +/-10000 limit, which clipped a unix "
     "timestamp",
     "Integer channels default to the full range. A test covers it"],
    ["Opening the Charts tab first showed 'no charts'",
     "The first chart was created by the Operator page, which had not drawn",
     "Created centrally, as soon as a node reports its schema"],
    ["The ESP32 would not join WiFi",
     "The uploaded file said 'Name: YOUR_WIFI_NAME', and the whole line was used "
     "as the network name",
     "The flash script strips 'Name:' and 'Password:' labels"],
    ["A stray scroll made a chart unreadable with no way back",
     "The plot was allowed to zoom and pan itself",
     "The chart sets its own bounds every frame; dragging pans, one button "
     "returns to live. Tests cover the limits"],
    ["Opening a profile mid-test disabled every board",
     "A file dialog blocks the thread that opens it, and a window that stops "
     "drawing is a fault",
     "Dialogs run on their own thread now; the fault rule stays"],
    ["Expo on a joystick seemed to do nothing",
     "It was applied inside the mapping, before the channel's range, and was "
     "never displayed",
     "Expo moved onto the channel, beside the value it affects. Tests check "
     "it damps small movements and still reaches full scale"],
    ["Analogue triggers always read zero",
     "On a DualShock they report as buttons with travel, not as axes",
     "Both are read, and buttons with travel are listed with the axes"],
    ["The Windows build showed empty boxes for arrows and ticks",
     "egui's built-in fonts do not carry them; Linux system fonts hid it",
     "Replaced with characters that always render"],
    ["The Windows build opened with no window on a machine with no GPU",
     "It defaulted to a graphics backend that needs one",
     "Tries OpenGL first, falls back automatically"],
    ["A second window made the machine stutter: 50 Hz fell to 4.4 Hz and the "
     "board's worst loop hit 2.02 seconds",
     "The board was sending telemetry to the watching station. The Ethernet "
     "chip resolves the address before every packet, and the watcher was a "
     "laptop on WiFi whose radio answers when it next wakes — 377 ms per "
     "frame",
     "The board only ever sends to a station already talking to it. The "
     "driving station forwards to watchers instead; a computer has an address "
     "cache and nothing else to do between frames"],
    ["Every page said it was waiting for the node's schema, with both boards "
     "powered and apparently fine",
     "Both boards were still holding Conformance.ino from the USB protocol "
     "tests. That sketch has no network at all, so nothing answered. The "
     "station was working perfectly and had nothing to show",
     "Reflashed SimNode and the WiFi node. The conformance command now prints "
     "the reflash lines when it finishes, and the station names the three "
     "things to check instead of only saying it is waiting"],
])
notes()

# ----------------------------------------------------------- 8. status

h("8. Status and what is next")

h("Done and verified on real hardware", 2)
bullets([
    "Protocol v2: numbered records, resending, station-set rate. 38 checks "
    "pass on both the Mega and the ESP32.",
    "Backfill over the network: 17 frames dropped on purpose, 17 records "
    "recovered, none lost.",
    "Cross-node links between two real boards: average error 0.0018, worst "
    "0.045, which is one frame of travel at 50 Hz.",
    "The ESP32 on WiFi at 192.168.0.51; all probe checks pass.",
    "Logging: a session exported at exactly 50 rows a second, rectangular, "
    "with no gaps. Start/stop and the CSV confirmed by hand.",
    "Failstates: a control falls back while the watchdog is tripped, and a "
    "control without one holds its last value.",
    "One station per board, watchers for the rest: with two stations running "
    "against both boards, both held 0.0% loss and both recorded complete logs "
    "at 50 Hz, while the board's worst loop stayed at 3.4 ms. Both boards "
    "pass 50 conformance checks.",
    "The Windows installer, tested under wine: silent install, four "
    "Start-menu shortcuts, the app runs, the uninstaller removes everything.",
    "102 automated tests.",
])

h("Done, but only seen under wine, not on Windows", 2)
bullets([
    "The Windows .exe opens, draws the whole window, and runs both links at "
    "0% loss. Wine is not Windows; the real check is still to come.",
])

h("Published", 2)
bullets([
    "Two public repositories in LostMachinist-Dev, pushed on 1 November 2026: "
    "microrio holds the station, the codec, the protocol spec, the library "
    "source and the docs; microrio-arduino holds the library at its own root, "
    "which is what the Arduino IDE needs in order to install it.",
    "GitHub sees one root commit per repository: the working history stays on "
    "this machine, on the local branches pre-publish-backup and master. "
    "Everything public is authored as lostmachinist. Later work goes on top as "
    "ordinary commits; what is published is never rewritten.",
    "The library is written in arduino/MicroRIO and pushed to its own "
    "repository by arduino/publish-library.sh. Never edit the published copy: "
    "that script overwrites it from this tree.",
    "The published trees have the home WiFi name and an old GitHub handle "
    "replaced with placeholders. The WiFi password was never in any commit.",
])

h("Open", 2)
bullets([
    "Running the installer and the app on a real Windows machine.",
])

h("Never tested", 2)
bullets([
    "The .exe on a real Windows machine.",
    "The ESP32 over Ethernet. It runs on WiFi; the PoE sketch compiles and "
    "passes over USB, but no cable has been plugged in.",
    "An Uno on the network. It is compile-only here — no Uno is attached.",
    "Three or more boards at once. Two have run together.",
])
notes()

OUT.parent.mkdir(parents=True, exist_ok=True)
doc.save(OUT)
print(f"wrote {OUT}")
