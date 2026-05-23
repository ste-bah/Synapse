use std::collections::BTreeMap;

use synapse_core::{Backend, Health, PerceptionMode, Point, Rect, Size};

#[test]
fn backend_json_shape() {
    insta::assert_json_snapshot!(Backend::Software);
    insta::assert_json_snapshot!(Backend::Vigem);
    insta::assert_json_snapshot!(Backend::Hardware);
    insta::assert_json_snapshot!(Backend::Auto);
}

#[test]
fn perception_mode_json_shape() {
    insta::assert_json_snapshot!(PerceptionMode::A11yOnly);
    insta::assert_json_snapshot!(PerceptionMode::PixelOnly);
    insta::assert_json_snapshot!(PerceptionMode::Hybrid);
    insta::assert_json_snapshot!(PerceptionMode::Auto);
}

#[test]
fn geometry_json_shape() {
    insta::assert_json_snapshot!(Point { x: -10, y: 20 });
    insta::assert_json_snapshot!(Rect {
        x: -10,
        y: 20,
        w: 30,
        h: 40,
    });
    insta::assert_json_snapshot!(Size { w: 1920, h: 1080 });
}

#[test]
fn health_json_shape() {
    insta::assert_json_snapshot!(Health {
        ok: true,
        version: "0.1.0".to_owned(),
        build: "dev".to_owned(),
        uptime_s: 0,
        subsystems: BTreeMap::new(),
    });
}
