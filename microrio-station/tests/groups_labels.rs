//! Value labels, header pins and groups. All profile-only: nothing here
//! touches the wire or the log.

#[path = "../src/config.rs"]
#[allow(dead_code)]
mod config;

use config::{label_text, ChanRef, ChannelConfig, Group, NodeConfig, ValueLabel, MAX_HEADER_ITEMS};

fn node_with_groups(names: &[&str]) -> NodeConfig {
    let mut n = NodeConfig::default();
    n.groups = names.iter().map(|s| Group::new(*s)).collect();
    n
}

fn label(value: i32, text: &str) -> ValueLabel {
    ValueLabel {
        value,
        label: text.into(),
        colour: None,
    }
}

#[test]
fn a_mapped_value_shows_its_label() {
    let table = vec![label(0, "auto"), label(1, "teleop")];
    assert_eq!(label_text(&table, 1), "teleop");
}

#[test]
fn an_unmapped_value_falls_back_to_the_number_never_blank() {
    let table = vec![label(0, "auto"), label(2, "")];
    assert_eq!(label_text(&table, 7), "7");
    // A row with an empty label is no better than no row.
    assert_eq!(label_text(&table, 2), "2");
    assert_eq!(label_text(&[], -3), "-3");
}

#[test]
fn label_colours_parse_with_or_without_the_hash() {
    let mut l = label(0, "open");
    l.colour = Some("#33aa55".into());
    assert_eq!(l.rgb(), Some([0x33, 0xaa, 0x55]));
    l.colour = Some("ff0000".into());
    assert_eq!(l.rgb(), Some([255, 0, 0]));
    l.colour = Some("red".into());
    assert_eq!(l.rgb(), None, "a bad colour is ignored, not an error");
}

#[test]
fn thresholds_warn_outside_the_band_only() {
    let c = ChannelConfig {
        warn_below: Some(11.5),
        ..Default::default()
    };
    assert!(c.warns(11.0));
    assert!(!c.warns(12.4));
    assert!(
        !ChannelConfig::default().warns(-1e9),
        "no threshold, no warning"
    );
}

#[test]
fn telemetry_may_sit_in_several_groups() {
    let mut n = node_with_groups(&["Valve 1", "Valve 2"]);
    let p = ChanRef::telemetry("pressure");
    n.place_in_group(0, p.clone(), None);
    n.place_in_group(1, p.clone(), None);
    assert_eq!(n.groups[0].members, vec![p.clone()]);
    assert_eq!(n.groups[1].members, vec![p]);
}

#[test]
fn a_control_dragged_into_a_second_group_moves() {
    let mut n = node_with_groups(&["Valve 1", "Valve 2"]);
    let open = ChanRef::control("open_cmd");
    n.place_in_group(0, open.clone(), None);
    n.place_in_group(1, open.clone(), None);
    assert!(n.groups[0].members.is_empty(), "left the first group");
    assert_eq!(n.groups[1].members, vec![open]);
}

#[test]
fn dragging_within_a_group_reorders_it() {
    let mut n = node_with_groups(&["g"]);
    let (a, b, c) = (
        ChanRef::telemetry("a"),
        ChanRef::telemetry("b"),
        ChanRef::control("c"),
    );
    for x in [&a, &b, &c] {
        n.place_in_group(0, x.clone(), None);
    }
    // Drop c onto a: c goes before a.
    n.place_in_group(0, c.clone(), Some(0));
    assert_eq!(n.groups[0].members, vec![c.clone(), a.clone(), b.clone()]);
    // Drop c onto b: removing c first shifts b up, and c still lands before it.
    n.place_in_group(0, c.clone(), Some(2));
    assert_eq!(n.groups[0].members, vec![a, c, b]);
}

#[test]
fn same_name_in_both_directions_is_two_channels() {
    let mut n = node_with_groups(&["g"]);
    n.place_in_group(0, ChanRef::telemetry("mode"), None);
    n.place_in_group(0, ChanRef::control("mode"), None);
    assert_eq!(n.groups[0].members.len(), 2);
}

#[test]
fn the_header_is_capped_counting_pinned_groups() {
    let mut n = node_with_groups(&["g"]);
    assert!(n.set_group_pinned(0, true));
    for i in 0..MAX_HEADER_ITEMS - 1 {
        assert!(n.set_pinned(&ChanRef::telemetry(format!("c{i}")), true));
    }
    assert_eq!(n.header_items(), MAX_HEADER_ITEMS);
    assert!(!n.set_pinned(&ChanRef::telemetry("one_too_many"), true));
    // Unpinning always works, and frees a slot.
    assert!(n.set_pinned(&ChanRef::telemetry("c0"), false));
    assert!(n.set_pinned(&ChanRef::telemetry("one_too_many"), true));
}

#[test]
fn groups_pins_and_labels_survive_a_save() {
    let mut n = node_with_groups(&["Valve 1"]);
    n.place_in_group(0, ChanRef::control("open_cmd"), None);
    n.groups[0].pinned = true;
    n.pins.push(ChanRef::telemetry("supply_v"));
    n.channels.insert(
        "supply_v".into(),
        ChannelConfig {
            units: "V".into(),
            warn_below: Some(11.5),
            ..Default::default()
        },
    );
    n.channels.insert(
        "mode".into(),
        ChannelConfig {
            labels: vec![label(0, "auto"), label(1, "teleop")],
            ..Default::default()
        },
    );
    let p = config::Profile {
        nodes: vec![n],
        ..Default::default()
    };
    let text = toml::to_string_pretty(&p).unwrap();
    let back: config::Profile = toml::from_str(&text).unwrap();
    let b = &back.nodes[0];
    assert_eq!(b.groups, p.nodes[0].groups);
    assert_eq!(b.pins, p.nodes[0].pins);
    assert_eq!(b.channels["supply_v"].warn_below, Some(11.5));
    assert_eq!(b.channels["mode"].labels.len(), 2);
}

#[test]
fn an_old_profile_without_any_of_this_still_loads() {
    let old = r#"
        [[nodes]]
        id = 1
        label = "mega"
        ip = "192.168.0.50"
        [nodes.channels.a0]
        units = "counts"
    "#;
    let p: config::Profile = toml::from_str(old).unwrap();
    assert!(p.nodes[0].groups.is_empty() && p.nodes[0].pins.is_empty());
}

#[test]
fn clock_plumbing_and_echoes_start_hidden_and_can_be_shown() {
    let mut n = NodeConfig::default();
    assert!(n.is_hidden(&ChanRef::control("set_unix")));
    assert!(n.is_hidden(&ChanRef::control("sync_time")));
    assert!(!n.is_hidden(&ChanRef::control("drive_x")));

    n.channels.insert(
        "mode_echo".into(),
        ChannelConfig {
            echo_of: Some("mode".into()),
            ..Default::default()
        },
    );
    assert!(n.is_hidden(&ChanRef::telemetry("mode_echo")));
    assert_eq!(n.echo_for("mode"), Some("mode_echo"));

    // An explicit choice wins over the default either way.
    n.channels.get_mut("mode_echo").unwrap().hidden = Some(false);
    assert!(!n.is_hidden(&ChanRef::telemetry("mode_echo")));
    n.controls.insert(
        "set_unix".into(),
        config::ControlConfig {
            hidden: Some(false),
            ..Default::default()
        },
    );
    assert!(!n.is_hidden(&ChanRef::control("set_unix")));
}

#[test]
fn a_profile_with_the_operator_chart_first_in_the_list_still_loads() {
    let old = r#"
        [[charts]]
        title = "live"
        [[charts]]
        title = "second"
    "#;
    let p: config::Profile = toml::from_str(old).unwrap();
    assert!(
        p.operator_chart.is_none(),
        "migrated by the window on first draw"
    );
    assert_eq!(p.charts.len(), 2);
}

#[test]
fn an_inverted_lamp_lights_on_false() {
    let plain = ChannelConfig::default();
    assert!(plain.lamp(true) && !plain.lamp(false));

    // rtc_ok and friends: true is the quiet state, false is the one to see.
    let inverted = ChannelConfig {
        invert_display: true,
        ..Default::default()
    };
    assert!(inverted.lamp(false), "false is what lights it");
    assert!(!inverted.lamp(true));
}

#[test]
fn display_settings_survive_a_save() {
    let mut n = NodeConfig::default();
    n.channels.insert(
        "rtc_ok".into(),
        ChannelConfig {
            invert_display: true,
            ..Default::default()
        },
    );
    let p = config::Profile {
        nodes: vec![n],
        operator_chart: Some(config::ChartConfig {
            invert_y: true,
            y_min: 0.0,
            y_max: 30.0,
            ..Default::default()
        }),
        ..Default::default()
    };
    let back: config::Profile = toml::from_str(&toml::to_string_pretty(&p).unwrap()).unwrap();
    assert!(back.nodes[0].channels["rtc_ok"].invert_display);
    let chart = back.operator_chart.unwrap();
    assert!(chart.invert_y, "a flipped Y axis is remembered");
    assert_eq!((chart.y_min, chart.y_max), (0.0, 30.0));
}

#[test]
fn a_profile_written_before_these_existed_still_loads() {
    let old = r#"
        [[nodes]]
        id = 1
        label = "n"
        ip = "192.168.0.50"
        [nodes.channels.rtc_ok]
        units = ""
        [[charts]]
        title = "c"
        y_min = -1.0
        y_max = 1.0
    "#;
    let p: config::Profile = toml::from_str(old).unwrap();
    assert!(!p.nodes[0].channels["rtc_ok"].invert_display);
    assert!(!p.charts[0].invert_y);
}
