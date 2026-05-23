use std::collections::BTreeMap;

use schemars::schema_for;
use synapse_core::{
    Backend, Health, PerceptionMode, Point, Rect, SCHEMA_VERSION, Size, SubsystemHealth,
    element_id, entity_id, new_reflex_id, new_session_id, new_subscription_id,
};

#[test]
fn backend_json_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let cases = [
        (Backend::Software, "\"software\""),
        (Backend::Vigem, "\"vigem\""),
        (Backend::Hardware, "\"hardware\""),
        (Backend::Auto, "\"auto\""),
    ];

    for (variant, json) in cases {
        assert_eq!(serde_json::to_string(&variant)?, json);
        assert_eq!(serde_json::from_str::<Backend>(json)?, variant);
    }

    assert!(serde_json::from_str::<Backend>("\"foo\"").is_err());
    assert!(serde_json::from_str::<Backend>("\"Software\"").is_err());
    assert!(serde_json::from_str::<Backend>("\"software \"").is_err());
    assert!(serde_json::from_str::<Backend>("null").is_err());
    Ok(())
}

#[test]
fn perception_mode_json_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let cases = [
        (PerceptionMode::A11yOnly, "\"a11y_only\""),
        (PerceptionMode::PixelOnly, "\"pixel_only\""),
        (PerceptionMode::Hybrid, "\"hybrid\""),
        (PerceptionMode::Auto, "\"auto\""),
    ];

    for (variant, json) in cases {
        assert_eq!(serde_json::to_string(&variant)?, json);
        assert_eq!(serde_json::from_str::<PerceptionMode>(json)?, variant);
    }

    assert!(serde_json::from_str::<PerceptionMode>("\"a11yOnly\"").is_err());
    assert!(serde_json::from_str::<PerceptionMode>("\"pixel\"").is_err());
    assert!(serde_json::from_str::<PerceptionMode>("\"\"").is_err());
    Ok(())
}

#[test]
fn geometry_json_and_helpers_are_stable() -> Result<(), Box<dyn std::error::Error>> {
    let point = Point { x: -100, y: 25 };
    let rect = Rect {
        x: 0,
        y: 0,
        w: 10,
        h: 10,
    };
    let size = Size { w: 1920, h: 1080 };

    assert_eq!(
        serde_json::from_str::<Point>(&serde_json::to_string(&point)?)?,
        point
    );
    assert_eq!(
        serde_json::from_str::<Rect>(&serde_json::to_string(&rect)?)?,
        rect
    );
    assert_eq!(
        serde_json::from_str::<Size>(&serde_json::to_string(&size)?)?,
        size
    );

    assert!(rect.contains(Point { x: 5, y: 5 }));
    assert!(!rect.contains(Point { x: 10, y: 5 }));
    assert!(
        !Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 10
        }
        .contains(Point { x: 0, y: 0 })
    );
    assert!(
        Rect {
            x: -200,
            y: -100,
            w: 50,
            h: 50,
        }
        .contains(Point { x: -175, y: -75 })
    );
    let distance = Point { x: 0, y: 0 }.distance_to(Point { x: 3, y: 4 });
    assert!((distance - 5.0).abs() < f64::EPSILON);
    Ok(())
}

#[test]
fn id_helpers_have_expected_shapes() {
    let first = new_session_id();
    let second = new_session_id();
    let uuid_v7 =
        regex::Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
            .unwrap_or_else(|err| panic!("regex should compile: {err}"));

    assert_ne!(first, second);
    assert!(uuid_v7.is_match(&first));
    assert!(uuid_v7.is_match(&new_reflex_id()));
    assert!(uuid_v7.is_match(&new_subscription_id()));
    assert_eq!(element_id(123, "abc"), "123:abc");
    assert_eq!(element_id(-1, "ff"), "-1:ff");
    assert_eq!(entity_id(42), "track:42");
    assert_eq!(entity_id(u64::MAX), format!("track:{}", u64::MAX));

    let ids: Vec<_> = (0..100).map(|_| new_session_id()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

#[test]
fn schema_version_is_root_reexported_u32() {
    let version: u32 = SCHEMA_VERSION;
    assert_eq!(version, 1);
}

#[test]
fn health_json_shape_and_schema_are_stable() -> Result<(), Box<dyn std::error::Error>> {
    let health = Health {
        ok: true,
        version: "0.1.0".to_owned(),
        build: "dev".to_owned(),
        uptime_s: 0,
        subsystems: BTreeMap::new(),
    };
    let expected = r#"{"ok":true,"version":"0.1.0","build":"dev","uptime_s":0,"subsystems":{}}"#;
    assert_eq!(serde_json::to_string(&health)?, expected);

    let value = serde_json::to_value(&health)?;
    assert_eq!(value["subsystems"], serde_json::json!({}));

    let schema = serde_json::to_value(schema_for!(Health))?;
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["subsystems"].is_object());

    let subsystem = SubsystemHealth {
        status: "healthy".to_owned(),
        detail: None,
    };
    assert_eq!(
        serde_json::to_value(subsystem)?,
        serde_json::json!({"status":"healthy","detail":null})
    );
    Ok(())
}
