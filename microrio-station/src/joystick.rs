//! Joystick input and live device view.
//!
//! This is a diagnostic instrument as much as a control source: it shows every
//! axis and button live so a controller can be validated on its own, with no
//! node connected and nothing mapped.
//!
//! Nothing here knows what a DualShock is. Controls that gilrs recognises get
//! their standard names (`LeftStickX`, `South`); anything else is listed by
//! the raw code the device reports, and maps just the same. That is what lets
//! a gamepad today and a purpose-built box tomorrow drive the same channel.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gilrs::ev::AxisOrBtn;
use gilrs::{Axis, Button, Event, EventType, Gilrs};

use crate::config::Mapping;
use crate::station::{JoyInput, Station};

/// Live state of one connected device.
#[derive(Debug, Clone, Default)]
pub struct DeviceState {
    pub connected: bool,
    /// Axis code to raw position, -1..1.
    pub axes: HashMap<String, f32>,
    /// Button code to pressed.
    pub buttons: HashMap<String, bool>,
    /// Button code to how far it is pressed, 0..1. A trigger such as L2 is a
    /// button with travel: on a DualShock it reports here, not as an axis,
    /// which is why its axis used to sit dead at zero.
    pub analog: HashMap<String, f32>,
    /// Codes this device reports that gilrs has no standard name for. They
    /// work like any other, but the name is platform-specific, so a profile
    /// using one may not carry to another machine.
    pub raw: Vec<String>,
}

#[derive(Debug, Default)]
pub struct JoystickState {
    pub devices: HashMap<String, DeviceState>,
    pub error: Option<String>,
}

/// Every control gilrs can describe. Polled in full, so the panel shows the
/// whole set from the moment a controller is plugged in.
///
/// Showing only controls that have already fired an event makes "this button is
/// broken" and "you have not pressed it yet" look identical, which is useless
/// in a tool whose job is telling those two apart.
const ALL_BUTTONS: [Button; 19] = [
    Button::South,
    Button::East,
    Button::North,
    Button::West,
    Button::C,
    Button::Z,
    Button::LeftTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger,
    Button::RightTrigger2,
    Button::Select,
    Button::Start,
    Button::Mode,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
];

const ALL_AXES: [Axis; 8] = [
    Axis::LeftStickX,
    Axis::LeftStickY,
    Axis::LeftZ,
    Axis::RightStickX,
    Axis::RightStickY,
    Axis::RightZ,
    Axis::DPadX,
    Axis::DPadY,
];

pub fn spawn(station: Arc<Station>) -> Arc<Mutex<JoystickState>> {
    let shared = Arc::new(Mutex::new(JoystickState::default()));
    let out = shared.clone();

    std::thread::spawn(move || {
        let mut gilrs = match Gilrs::new() {
            Ok(g) => g,
            Err(e) => {
                shared.lock().unwrap().error = Some(format!("no joystick support: {e}"));
                return;
            }
        };

        loop {
            // gilrs only updates its internal state while events are drained,
            // so pump first and poll after.
            while gilrs.next_event().is_some() {}

            {
                let mut s = shared.lock().unwrap();
                s.devices.clear();
                for (_, pad) in gilrs.gamepads() {
                    let mut dev = DeviceState {
                        connected: pad.is_connected(),
                        ..Default::default()
                    };
                    // The standard set first, so a recognised controller
                    // lists every control it has even before one is touched.
                    for b in ALL_BUTTONS {
                        dev.buttons.insert(button_code(b), pad.is_pressed(b));
                        if let Some(d) = pad.button_data(b) {
                            dev.analog.insert(button_code(b), d.value());
                        }
                    }
                    for a in ALL_AXES {
                        dev.axes.insert(axis_code(a), pad.value(a));
                    }

                    // Then whatever else this device actually reports. A
                    // control with no standard name is listed by its raw code
                    // rather than left out, which is the whole difference
                    // between supporting gamepads and supporting controllers.
                    let st = pad.state();
                    for (code, data) in st.axes() {
                        let name = match pad.axis_or_btn_name(code) {
                            Some(AxisOrBtn::Axis(a)) => axis_code(a),
                            Some(AxisOrBtn::Btn(b)) => button_code(b),
                            None => {
                                let raw = code.to_string();
                                dev.raw.push(raw.clone());
                                raw
                            }
                        };
                        dev.axes.insert(name, data.value());
                    }
                    for (code, data) in st.buttons() {
                        let name = match pad.axis_or_btn_name(code) {
                            Some(AxisOrBtn::Btn(b)) => button_code(b),
                            Some(AxisOrBtn::Axis(a)) => axis_code(a),
                            None => {
                                let raw = code.to_string();
                                dev.raw.push(raw.clone());
                                raw
                            }
                        };
                        dev.analog.insert(name.clone(), data.value());
                        dev.buttons.insert(name, data.is_pressed());
                    }
                    dev.raw.sort();
                    dev.raw.dedup();
                    s.devices.insert(pad.name().to_string(), dev);
                }
            }

            apply_mappings(&station, &shared);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });

    out
}

/// Find the configured device entry that belongs to a connected controller.
///
/// gilrs names the same pad differently per platform — a DualShock 4 is
/// "PS4 Controller" on Linux and reports differently on Windows — so a profile
/// carried between machines would otherwise silently stop driving anything.
/// Exact match first, then case-insensitive, then: if there is exactly one
/// configured device and one controller plugged in, they must be each other.
pub fn match_device<'a>(
    connected: &str,
    connected_count: usize,
    configured: &'a [String],
) -> Option<&'a String> {
    if let Some(hit) = configured.iter().find(|c| c.as_str() == connected) {
        return Some(hit);
    }
    if let Some(hit) = configured
        .iter()
        .find(|c| c.eq_ignore_ascii_case(connected))
    {
        return Some(hit);
    }
    if configured.len() == 1 && connected_count == 1 {
        return configured.first();
    }
    None
}

/// Push conditioned axis and button values into the control intents.
fn apply_mappings(station: &Arc<Station>, shared: &Arc<Mutex<JoystickState>>) {
    let devices = {
        let s = shared.lock().unwrap();
        s.devices.clone()
    };
    let configured: Vec<String> = {
        let p = station.profile.lock().unwrap();
        p.joystick.devices.keys().cloned().collect()
    };

    // Resolve each connected pad to a configured entry, then collect its
    // mappings. Driven from the connected side so a renamed device still works.
    let mut mappings: Vec<(String, Mapping)> = Vec::new();
    {
        let p = station.profile.lock().unwrap();
        for name in devices.keys() {
            let Some(key) = match_device(name, devices.len(), &configured) else {
                continue;
            };
            if let Some(cfg) = p.joystick.devices.get(key) {
                for m in &cfg.mappings {
                    mappings.push((name.clone(), m.clone()));
                }
            }
        }
    }

    let mut out: HashMap<u16, HashMap<String, JoyInput>> = HashMap::new();
    for (dev_name, m) in mappings {
        let Some(dev) = devices.get(&dev_name) else {
            continue;
        };
        if !dev.connected {
            continue;
        }
        if let Some((raw, unipolar)) = read_source(dev, &m.source) {
            out.entry(m.node_id).or_default().insert(
                m.channel.clone(),
                JoyInput {
                    source: m.source.clone(),
                    deflection: m.deflection(raw, unipolar),
                    unipolar,
                },
            );
        }
    }
    *station.joy.lock().unwrap() = out;
}

/// The current reading of an axis or button, and whether it is one-sided.
///
/// A button with travel reports how far it is pressed; a plain button is 0 or
/// 1. Either way it rests at 0, unlike a stick, which rests in the middle.
pub fn read_source(dev: &DeviceState, source: &str) -> Option<(f32, bool)> {
    if let Some(v) = dev.axes.get(source) {
        return Some((*v, false));
    }
    if let Some(v) = dev.analog.get(source) {
        return Some((*v, true));
    }
    dev.buttons
        .get(source)
        .map(|pressed| (if *pressed { 1.0 } else { 0.0 }, true))
}

pub fn axis_code(a: Axis) -> String {
    format!("{a:?}")
}

pub fn button_code(b: Button) -> String {
    format!("{b:?}")
}

/// Print what gilrs can see, live, for a few seconds. Headless, so it works
/// over ssh and answers "is the controller the problem?" without the window.
pub fn list_headless(seconds: u64) -> anyhow::Result<()> {
    let mut gilrs = Gilrs::new().map_err(|e| anyhow::anyhow!("no joystick support: {e}"))?;

    let pads: Vec<_> = gilrs
        .gamepads()
        .map(|(_, p)| p.name().to_string())
        .collect();
    if pads.is_empty() {
        println!("No controllers detected.");
        println!("If one is plugged in, check that /dev/input/js0 exists and is readable.");
        return Ok(());
    }
    for name in &pads {
        println!("Controller: {name}");
    }
    println!("\nWatching for {seconds}s — move sticks and press buttons.\n");

    let mut axes: HashMap<String, f32> = HashMap::new();
    let mut seen_buttons: Vec<String> = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);

    while std::time::Instant::now() < deadline {
        while let Some(Event { event, .. }) = gilrs.next_event() {
            match event {
                EventType::AxisChanged(axis, v, _) => {
                    axes.insert(axis_code(axis), v);
                }
                EventType::ButtonPressed(btn, _) => {
                    let c = button_code(btn);
                    println!("  button pressed: {c}");
                    if !seen_buttons.contains(&c) {
                        seen_buttons.push(c);
                    }
                }
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    println!("\nAxes reporting ({}):", axes.len());
    let mut sorted: Vec<_> = axes.iter().collect();
    sorted.sort_by_key(|(k, _)| (*k).clone());
    for (code, v) in sorted {
        println!("  {code:<16} {v:+.3}");
    }
    println!("\nButtons seen pressed ({}):", seen_buttons.len());
    for b in &seen_buttons {
        println!("  {b}");
    }
    if seen_buttons.is_empty() {
        println!("  (none — nothing was pressed during the watch)");
    }

    println!("\nEvery control this controller exposes:");
    for (_, pad) in gilrs.gamepads() {
        for b in ALL_BUTTONS {
            if pad.is_pressed(b) {
                println!("  button {:<16} PRESSED", button_code(b));
            }
        }
        for a in ALL_AXES {
            println!("  axis   {:<16} {:+.3}", axis_code(a), pad.value(a));
        }
    }
    println!("\nUse these exact codes as the 'source' in a mapping.");
    Ok(())
}
