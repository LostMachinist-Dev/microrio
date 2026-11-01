# Running the MicroRIO driver station

Two programs, same app:

- `microrio-station` — Linux, runs on serverbox.
- `microrio-station.exe` — Windows, runs on your laptop. No installer, no
  dependencies; it is one file.

The window opens whether or not a board is connected, so you can look around
first. The joystick tab works with nothing attached but a controller.

---

## On the box

Run in: box (yerkovich)
```
~/deliverables/microrio/microrio-station
```

Opens the driver station window.

You should see a window titled **MicroRIO Driver Station** with a grey
**DISABLED** badge top-left and four tabs: Nodes, Joystick, Logging, Settings.
Under Nodes it will say *Waiting for the node's schema…* because no board is
talking yet. That is correct.

If nothing opens, the box has no screen attached at that moment — use the
Windows copy instead.

---

## On the laptop

1. Open http://192.168.0.148:8000/deliverables/microrio in a browser and
   download `microrio-station.exe`.

2. Double-click it.

Windows will likely warn that it is from an unknown publisher. Click
**More info**, then **Run anyway**. It is unsigned because nobody has paid for
a code-signing certificate, not because anything is wrong with it.

Same window, same four tabs.

---

## Pointing it at a board

The station does not search the network. You type the board's address in, once,
and it is saved.

1. Open the **Settings** tab.
2. Set **ip** to the address in the sketch — `192.168.0.50` for the Mega
   example, `192.168.0.51` for the ESP32 one.
3. Click **Save profile**, then close and reopen the app.

Node changes only take effect on restart. Everything else — joystick mappings,
enable, e-stop — is live.

You should see the circle next to the node name fill in, and the Nodes tab
replace *Waiting for the node's schema…* with two columns of live values.

If it stays hollow, the board and the box are not on the same network yet.

---

## The Operator page

This is the page you will live on. It has three things at once, because
watching a number and watching its shape are the same job:

- **Telemetry** is what the board reports. You can only read it.
- **Control** is what you send. Nothing fills these in but you, a joystick
  mapping, or a cross-node link. A control channel with nothing driving it
  sits where you left it. That is not a bug.
- **The chart** underneath, so you are not switching tabs to see a trace.

Make the window wide and telemetry and control sit side by side. Make it
narrow — half a screen — and they stack instead, and you scroll. Both shapes
are meant to work; nothing should ever overlap. If it does, tell me the window
size and I will fix it.

---

## Enable, disable, e-stop

**Enable is per board. E-stop is global.**

Each board has its own row near the top of the window with its own **ENABLE**
and **DISABLE** buttons. One board can be driving while another sits parked.
The badge top-left tells you the worst thing currently true across all of
them, and next to it "2 of 3 enabled" tells you how many are live.

**E-STOP** is the big red one, and it is the only thing that acts on
everything at once. It latches. **Clear E-Stop** is the only way out, and it
leaves every board disabled rather than handing them back to you enabled.

**Disable all** is the calm version of the same thing: everything off, nothing
latched.

There are four separate ways to end up stopped:

- You press **E-STOP**.
- You disable that board.
- The window stops drawing — minimised too long, dragged, or the machine too
  busy. The station notices within 250 ms and faults every board. File dialogs
  are not one of these: they open on their own thread, so Open, Save as and
  CSV export leave the station running and driving.
- The board stops hearing from the station for 200 ms. It falls back to its
  failstates and faults itself, without waiting to be told.

### If a node says SECOND STATION or NOT IN CONTROL

A board accepts control from one station at a time.

- **SECOND STATION** — you are driving this board, and another station is
  trying to. Your link is unaffected; theirs is being refused. Find the other
  station and close it.
- **VIEW ONLY** — someone else is driving this board. You see everything it
  reports, at full rate, and you can chart and record it; you cannot set
  anything. **The E-stop still works** — a watcher can always stop the
  machine, and the station driving it cannot override that. Press **take
  control** to ask for the board; you get it once the other station stops
  talking to it.

  What you are watching comes from the station that is driving, not from the
  board: it forwards what it receives. So a watcher shows no round trip — it
  never speaks to the board — and it goes blank if that station closes. If a
  watcher stays empty, check that the driving station has relaying on
  (Settings) and that both are on the same network.
- **NOT IN CONTROL** — the same thing, before the station has settled into
  watching. It clears itself within a second or two.

If a board's rate collapses with a watcher connected — and it is an Ethernet
shield — check the sketch calls `Ethernet.setRetransmissionTimeout(50)` and
`setRetransmissionCount(1)`. Without them, sending to an address that does not
answer blocks the board for over a second at a time. The library drops such a
watcher by itself, but those two lines are what make it cheap.

Opening the station twice on one computer is refused outright, with a message
saying so. If you want a second one — on another machine, or deliberately
here with `--allow-multiple` — it comes up as a watcher. `--view` starts one
that never drives anything, which is what you want on a screen somebody is
only looking at.

The probe is a station too: stop the app before running `cargo xtask probe`,
or it will tell you the node belongs to someone else.

### If the window stalls

A stall now **leaves everything disabled**, and puts an orange banner across
the top saying how long it was out for. You dismiss the banner and re-enable
deliberately.

This changed on purpose. Before, a stall faulted the boards and then quietly
handed them back as soon as the window drew again — so anything that blocked
the window could stop the machine and restart it without telling you. The
window coming back does not mean you are watching again.

---

## Logs

**Everything is always being recorded.** From the moment the app opens to when
you quit, every telemetry frame and every control frame goes to one file:

```
~/.local/share/microrio/logs/
```

on the box, or `%APPDATA%\microrio\logs\` on Windows. You do not have to
remember to press anything, and you cannot lose a run by forgetting to.

### Start and stop

**Start Data Log** and **Stop Data Log** on the Logging tab mark the stretch
you actually care about. The clock beside the button counts while it runs.

When you press stop, a normal Save box opens and you choose where the CSV
goes. Only the stretch between start and stop is written to it. Cancel the
Save box and nothing is lost — it is all still in the background file.

One board gives you exactly the file you named. A second board lands beside
it as `<yourname>-node2.csv`.

### What is in the CSV

One row per telemetry frame, with the control values that were current at that
moment. Two time columns: `time_us`, microseconds since the app opened, and
`time_local`, the time of day in your zone.

### Exporting an old session

**Export whole session…** on the Logging tab does the same thing for the
entire file rather than a marked stretch.

Or without opening the window at all:

Run in: box (yerkovich)
```
~/deliverables/microrio/microrio-station export ~/.local/share/microrio/logs/<file>.mrlog
```

Writes the CSVs next to the log file and prints where they went.

---

## Your settings file

Everything you change in Settings and Joystick is saved here:

```
~/.config/microrio/station.toml
```

on the box, or `%APPDATA%\microrio\station.toml` on Windows. It is plain text,
so you can read it, edit it, and copy it between the two machines.

---

## Installing the Arduino library

`MicroRIO-0.1.0.zip` is laid out so it drops straight into your sketchbook.

Run in: box (yerkovich)
```
unzip ~/deliverables/microrio/MicroRIO-0.1.0.zip -d ~/Arduino/libraries/
```

On Windows, extract it into `Documents\Arduino\libraries\` instead — or use
the IDE's *Sketch → Include Library → Add .ZIP Library…*, which does the same
thing.

You should end up with `libraries/MicroRIO/src/MicroRIO.h`, and after an IDE
restart MicroRIO appears under *Include Library*, with the four examples under
*File → Examples → MicroRIO*.

Start with **SimNode** — it generates sine, ramp, square and noise and drives
the DS3231, so there is something worth looking at on the chart.

There is no `.cpp`. The library is a template, so capacity is fixed where you
declare it and you only pay for the channels you ask for; C++ requires template
code to live in the header. `MicroRIO_protocol.h` beside it is **generated**
from `spec/protocol.toml`, which also generates the Rust side — so the firmware
and the station cannot drift apart. Change the spec, not that file.

---

## Settings files

The bar under the safety row handles them:

- **Open…** — pick any `.toml` anywhere. The links rebuild immediately, so it
  takes effect at once rather than needing a restart.
- **Save** — writes back to the file that is open.
- **Save as…** — writes somewhere new, and that becomes the live file, so the
  next Save goes where you expect.
- **Reload** — throws away unsaved edits and re-reads from disk.

A profile holds everything: the node list, your time zone, per-channel
units/slope/offset, your charts and their axes, cross-node links, control
step/min/max/offset, and joystick mappings.

**It does not hold which boards are enabled.** That is deliberate: a file must
never be able to decide what is live when it is opened.

If you have an older profile, the `enabled` key under `[[nodes]]` is still
read, and now means "open a link to this board at all" — it is the **link**
tick on the Settings page, not the enable switch. It is written out as
`link_active` the next time you save.

Because it is just a file, one copy of the app can serve several machines —
keep `shop-bench.toml`, `robot-a.toml`, `field.toml` side by side and open
whichever you need.

You can also start with one directly:

Run in: box (yerkovich)
```
~/deliverables/microrio/microrio-station --profile ~/profiles/robot-a.toml
```

With no `--profile` it uses the per-user default, which lives at
`~/.config/microrio/station.toml` on Linux and `%APPDATA%\microrio\station.toml`
on Windows.

**Opening a profile always leaves every board disabled.** Loading someone
else's settings should never be able to start a machine moving.

One thing to know about carrying a profile between machines: the joystick is
named differently by the OS — a DualShock 4 is "PS4 Controller" on Linux and
reports differently on Windows. The app matches by name, then ignoring case,
and finally assumes a single configured pad and a single plugged-in pad are
each other. With two pads and no name agreement it does nothing rather than
guess, so re-add the mapping on that machine.

---

## The telemetry and control grids

Each telemetry row shows **raw** and **scaled** side by side, with **slope**,
**offset** and **units** you can edit in place. Calibration lives here, not in
the firmware, so correcting a scale factor is a change you make while watching
the number move rather than a reflash.

    scaled = raw x slope + offset

The **+Y** button in the last column drops that channel onto the chart below,
or takes it off again.

Numeric controls get the full set: a **[- value +]** box with increment
buttons, plus **step**, **min**, **max** and **offset**. The buttons move by
one step and stop at the limits.

### The wire column

The **wire** column is what actually leaves the station: your value, plus the
offset, then clamped to min and max. The value column is what you asked for;
the wire column is what the board gets.

Two things worth knowing about it:

- Until now the wire column was **wrong** whenever offset was not zero. It
  showed the offset applied, but the offset was never applied to the frame.
  It is applied now, so the two agree.
- Because min and max really are enforced, a channel you have given limits to
  sends its minimum rather than zero when you have not touched it. `freq_hz`
  with min 0.05 shows value `0.00` and wire `0.050`, and 0.05 is what goes
  out. That is the limits doing their job.

Integer channels have no limits unless you set them, so `set_unix` can still
carry a real timestamp.

Bools are a lamp you click. Commands are a Fire button with the counter beside
it, so you can see it increment.

A channel driven by a cross-node link is marked in orange and goes read-only.

---

## Charts

The chart sits on the Operator page. The **Charts** tab holds that same chart
plus any others you add; the first one in the list is always the Operator one.

### Choosing what goes on which axis

Instead of arming channels, you say what each axis is. Two dropdowns:

- **X** — `time · seconds ago` (the usual scope view, newest at 0 on the
  right, scrolling left), `time · clock` (actual time of day), or any
  telemetry channel on any board.
- **Y** — one or more channels. **+** adds a trace, **×** removes one.

Put a channel on X instead of time and you get an **X-Y plot**: one value
against another rather than against time. Those are drawn as points, not a
joined line, because joining them draws loops that are an artefact of the
order the samples arrived in.

**⇄ flip** swaps X with the first Y trace. Time is allowed on either axis, so
flipping a normal trace just stands it on its side.

Channels from two different boards can share a chart. They are matched by
time: each Y sample takes the most recent X sample at or before it. If the X
source has been quiet for more than half a second, nothing is drawn rather
than a flat line out of stale data.

### Scaling

Traces are the **scaled** values, not raw. That matters: a channel reading 632
counts and one swinging between -1 and 1 cannot share an axis usefully. Give
the big one a slope — `a0` with slope 0.002 and offset -1 drops it onto the
same axis as the waveforms. That is what slope and offset are for here.

---

## Time, and the clock

### Setting your zone

**Settings → Time zone**. Leave it on `system` and it follows whatever the
machine is set to, which is what you want on a laptop. Pick a named zone and
it is pinned, which is what you want when your logs have to line up with
someone else's.

Every time in the app uses it: the clock panel, a chart's clock axis, and the
`time_local` column in an exported CSV. **There is no UTC anywhere in the
window any more.** The wire never carries a zone at all — the board keeps a
plain count of seconds, which is exactly why its clock can be shown in yours.

### The clock tab

Station time and node time, both in your zone, with the **drift** between them
in seconds.

**Sync node clock to station** sets `set_unix` and then increments the
`sync_time` counter. A counter rather than a pulse, so a dropped packet delays
the sync instead of losing it.

A node only gets this panel if it declares `unix_time` telemetry plus
`set_unix` and `sync_time` controls. `SimNode` does.

One leftover: `rtc_time` in the telemetry grid is a string the board formats
itself, and the board formats it in UTC. It is the board's own text, not the
station's. The Clock tab is the one to trust.

---

## Driving one board from another

**Settings → Cross-node links.** Take a reading from one board and use it to
drive an output on another:

    from node 1, telemetry a0  ->  to node 2, control setpoint
    scaled by slope and offset, clamped to min and max

The channel on the receiving board is then marked orange in the control grid
and stops being yours to set by hand.

**If the source board goes quiet for a fifth of a second, the board being
driven is put into FAULT.** It is not held at the last value it saw. A dead
sensor must not go on commanding an output, and you will see `LINK SOURCE
LOST` on that board's row.

---

## Frequency, and what it actually changes

The **tx** box on each board's row is how fast the station sends control
frames to it, and therefore how fast the control side of the log fills. You
can change it while running; it takes effect immediately, with no restart and
no re-handshake.

Next to it, **node NN Hz** is how fast that board sends telemetry. That one is
compiled into the sketch with `node.rate(50)` and the station cannot change
it. If the two differ the box turns orange, because then telemetry and control
are arriving at different rates and the log is not square.

So they are **not** one setting today. Making them one means adding a field to
the control frame and reflashing every board — worth doing, not done yet.

---

## Checking a controller

The joystick tab shows every axis and every button the controller has, from the
moment you plug it in — not just the ones you have already touched. Press a
button and watch it light up. If a button never lights, that is a real answer,
because the rest of them do.

There is also a headless version, for when you are on ssh with no window:

Run in: box (yerkovich)
```
~/deliverables/microrio/microrio-station joysticks 10
```

Watches for ten seconds and prints every button press as it happens, then lists
every control the pad exposes with its resting value.

You should see the controller named, presses echoed as you make them, and a
full list of axes at the end.

If it says no controllers detected, check that `/dev/input/js0` exists.

### Mapping an axis to a channel

Use the exact code from that list as the **source** — `LeftStickX`, `South`,
`RightTrigger2` and so on. Pick the node and type the control channel name,
then press **Add mapping**.

Sticks drift. The one here rests at about -0.008 on RightStickX, which is
enough to creep an actuator, so give every axis mapping a small **deadband** —
0.05 is a sensible start. Past the deadband the output starts from zero rather
than jumping, so nothing lurches when the stick comes alive.

**trim** shifts where neutral sits, **expo** softens the middle without
reducing full deflection, and **min**/**max** set the range the channel sees.

---

## Opening straight to a tab, or at a set size

Run in: box (yerkovich)
```
~/deliverables/microrio/microrio-station --tab joystick
```

Opens on that tab instead of Operator. Also accepts `operator`, `charts`,
`clock`, `logging`, `settings`.

Run in: box (yerkovich)
```
~/deliverables/microrio/microrio-station --size 1920x1080
```

Opens the window at that size. Handy for checking a layout at a size you are
about to complain about.

---

## If a node will not connect

There is a headless probe that talks to a board directly and says exactly where
it breaks — no window needed, so it works over ssh.

Run in: box (yerkovich)
```
cd ~/projects/microrio && cargo xtask probe 192.168.0.50
```

Prints the board's full channel list, drives it for a second and a half, then
goes quiet to confirm the board faults itself.

You should see the channel list, then a run of `ok` lines ending in
**all probe checks passed**.

If the schema line never appears, the board is not reachable: check `ping
192.168.0.50` first.

### "No answer from 192.168.0.50:5005"

Every page says this, with the number of times it has asked, when nothing at
all comes back. The station is working and has nothing to draw: a node is
asked what channels it has and will not be guessed at. Three things cause it,
all of them at the board's end.

1. **The board is switched off**, or its network cable is out.
2. **It is at a different address** from the one in Settings -> Nodes.
3. **It is running a sketch that is not a MicroRIO node.** The easiest way
   into this one is the protocol test: `cargo xtask conformance` leaves the
   board holding `Conformance.ino`, which speaks over USB and never touches
   the network. Reflash the node sketch and it comes straight back.

Run in: box (yerkovich)
```
cd ~/projects/microrio
arduino-cli compile -b arduino:avr:mega --library arduino/MicroRIO \
    -u -p /dev/ttyACM0 arduino/MicroRIO/examples/SimNode
arduino/flash-wifi-node.sh /dev/ttyUSB0
```

If chunks are arriving, the message says so and counts them: that is the
handshake running, not a fault.

### "only N% arrived"

Both the probe and the node bar in the app report how much of what the board
sent actually reached you, counted from gaps in its sequence numbers.

Below 90% used to mean a second copy of the station was running. A board now
answers to one station at a time and says so — **SECOND STATION** if you own
it, **NOT IN CONTROL** if you do not — so that cause names itself rather than
looking like a bad network. An older board, flashed before this, still splits
its telemetry; the station works that out from whose clock comes back and
reports it the same way.

If you are certain only one station is running, then it really is the network.

---

## Why a pin with nothing on it still shows a number

`a0` in SimNode is a real ADC read, and with nothing wired to it the pin is
**floating**. A floating input has no defined voltage — it drifts to wherever
leakage and stray capacitance leave it. On this Mega it sits near 580 counts
and barely moves, which looks convincingly like a real measurement.

Measured on the bench: two unconnected pins read 580 and 572, within 8 counts
of each other, and swapping which is read first changes nothing. They are not
influencing each other; they simply settle to nearly the same place because
they share a package and an environment. Drive one of them low and it reads 0
immediately, which is the proof there is no real source behind the number.

So: wire something to a pin, or tie it to GND, before believing what it reads.

`battery_v` used to read A1 for this reason and looked like it agreed with
`a0`. It is now a simulated pack voltage, which is honest about what it is.

---

## A note on the round-trip number

The `rtt` in the app is the age of your last control frame as the board saw it,
not a pure network ping. It includes up to one of the board's own send periods —
20 ms at 50 Hz — so on a quiet LAN expect roughly 10–30 ms rather than
sub-millisecond. A sudden jump still means what you would expect; the baseline
is just not zero.

It shows `—` until the board has echoed a timestamp that came from this session.
