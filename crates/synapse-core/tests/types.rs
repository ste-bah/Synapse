use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use proptest::{
    prelude::*,
    test_runner::{Config, TestRng, TestRunner},
};
use schemars::schema_for;
use synapse_core::{
    AccessibleNode, AccessibleQuery, AccessibleQueryScope, AudioContext, Backend, ClipboardSummary,
    DataPredicate, DetectedEntity, Detection, DetectionBatch, ElementId, Event, EventFilter,
    EventRef, EventSource, FocusedElement, ForegroundContext, FsEvent, FsEventKind, Health,
    HudReading, HudReadings, HudValue, Observation, ObservationDiagnostics, PerceptionMode, Point,
    Rect, SCHEMA_VERSION, SensorStatus, Size, SubsystemHealth, UiaPattern, element_id, entity_id,
    new_reflex_id, new_session_id, new_subscription_id,
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
    assert_eq!(element_id(123, "abc"), "0x7b:abc");
    assert_eq!(element_id(-1, "ff"), "-0x1:ff");
    assert_eq!(entity_id(42), "track:42");
    assert_eq!(entity_id(u64::MAX), format!("track:{}", u64::MAX));

    let ids: Vec<_> = (0..100).map(|_| new_session_id()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

#[test]
fn element_id_parse_display_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let first = ElementId::parse("0x12ab:0a1b2c3d")?;
    let second = ElementId::parse("0x12ab:ffffffff")?;
    let first_parts = first.parts()?;

    assert_eq!(first.to_string(), "0x12ab:0a1b2c3d");
    assert_eq!(first_parts.hwnd, 0x12ab);
    assert_eq!(first_parts.runtime_id_hex, "0a1b2c3d");
    assert_ne!(first, second);
    assert_eq!(serde_json::to_string(&first)?, "\"0x12ab:0a1b2c3d\"");
    assert_eq!(
        serde_json::from_str::<ElementId>("\"0x12ab:0a1b2c3d\"")?,
        first
    );

    assert!(serde_json::from_str::<ElementId>("\"not-an-element\"").is_err());
    assert!(ElementId::parse("0x12ab").is_err());
    assert!(ElementId::parse("12ab:0a1b").is_err());
    assert!(ElementId::parse("0x12ab:").is_err());
    assert!(ElementId::parse("0x12ab:not-hex").is_err());
    Ok(())
}

#[test]
fn accessible_query_defaults_to_focused_subtree() -> Result<(), Box<dyn std::error::Error>> {
    let query = serde_json::from_value::<AccessibleQuery>(serde_json::json!({
        "role": "Button",
        "name_substring": "Save"
    }))?;

    assert_eq!(query.scope, AccessibleQueryScope::FocusedSubtree);
    assert_eq!(query.role.as_deref(), Some("Button"));
    assert_eq!(query.name_substring.as_deref(), Some("Save"));
    assert!(query.automation_id.is_none());
    assert!(
        serde_json::from_value::<AccessibleQuery>(serde_json::json!({
            "role": "Button",
            "unexpected": true
        }))
        .is_err()
    );
    Ok(())
}

#[test]
fn observation_json_round_trips_and_preserves_ids() -> Result<(), Box<dyn std::error::Error>> {
    let observation = sample_observation()?;
    let json = serde_json::to_string(&observation)?;
    let parsed = serde_json::from_str::<Observation>(&json)?;

    assert_eq!(parsed, observation);
    assert_eq!(
        parsed.focused.as_ref().map(|focused| &focused.element_id),
        Some(&element_id(0x12ab, "0a1b2c3d"))
    );
    assert_eq!(
        serde_json::from_str::<Observation>(
            r#"{"seq":1,"at":"2026-05-23T00:00:00Z","mode":"auto","extra":true}"#
        )
        .map(|value| value.seq)
        .ok(),
        None
    );
    Ok(())
}

#[test]
fn detection_batch_json_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let batch = DetectionBatch {
        model_id: "yolov10n-general".to_owned(),
        frame_seq: 42,
        inferred_at: fixed_time()?,
        items: vec![Detection {
            class_label: "enemy".to_owned(),
            bbox: Rect {
                x: 10,
                y: 20,
                w: 30,
                h: 40,
            },
            confidence: 0.875,
            track_id: Some(7),
        }],
    };

    assert_eq!(
        serde_json::from_str::<DetectionBatch>(&serde_json::to_string(&batch)?)?,
        batch
    );
    Ok(())
}

#[test]
fn event_filter_predicates_cover_each_variant() -> Result<(), Box<dyn std::error::Error>> {
    let event = sample_event()?;

    assert!(EventFilter::All.matches(&event));
    assert!(!EventFilter::None.matches(&event));
    assert!(
        EventFilter::Kind {
            kind: "hud-value-changed".to_owned()
        }
        .matches(&event)
    );
    assert!(
        EventFilter::Source {
            source: EventSource::PerceptionHud
        }
        .matches(&event)
    );
    assert!(
        EventFilter::And {
            args: vec![
                EventFilter::Data {
                    path: "/field".to_owned(),
                    predicate: DataPredicate::Eq {
                        value: serde_json::json!("hp"),
                    },
                },
                EventFilter::Data {
                    path: "/new".to_owned(),
                    predicate: DataPredicate::Lt {
                        value: serde_json::json!(20),
                    },
                },
            ],
        }
        .matches(&event)
    );
    assert!(
        EventFilter::Or {
            args: vec![
                EventFilter::Kind {
                    kind: "focus-changed".to_owned(),
                },
                EventFilter::Kind {
                    kind: "hud-value-changed".to_owned(),
                },
            ],
        }
        .matches(&event)
    );
    assert!(
        EventFilter::Not {
            arg: Box::new(EventFilter::Kind {
                kind: "focus-changed".to_owned(),
            }),
        }
        .matches(&event)
    );

    assert!(DataPredicate::Exists.matches(event.data.pointer("/field")));
    assert!(
        DataPredicate::Ne {
            value: serde_json::json!("ammo")
        }
        .matches(event.data.pointer("/field"))
    );
    assert!(
        DataPredicate::Le {
            value: serde_json::json!(19)
        }
        .matches(event.data.pointer("/new"))
    );
    assert!(
        DataPredicate::Gt {
            value: serde_json::json!(5)
        }
        .matches(event.data.pointer("/new"))
    );
    assert!(
        DataPredicate::Ge {
            value: serde_json::json!(15)
        }
        .matches(event.data.pointer("/new"))
    );
    assert!(
        DataPredicate::Regex {
            pattern: "^h.$".to_owned()
        }
        .matches(event.data.pointer("/field"))
    );
    assert!(
        DataPredicate::InSet {
            values: vec![serde_json::json!("ammo"), serde_json::json!("hp")]
        }
        .matches(event.data.pointer("/field"))
    );
    assert!(
        !DataPredicate::Regex {
            pattern: "[".to_owned()
        }
        .matches(event.data.pointer("/field"))
    );
    Ok(())
}

#[test]
fn observation_proptest_json_round_trip_is_deterministic() -> Result<(), Box<dyn std::error::Error>>
{
    let config = Config {
        cases: 1_000,
        failure_persistence: None,
        ..Config::default()
    };
    let algorithm = config.rng_algorithm;
    let mut runner = TestRunner::new_with_rng(config, TestRng::deterministic_rng(algorithm));

    runner.run(&observation_strategy(), |observation| {
        let json = serde_json::to_string(&observation)?;
        let parsed = serde_json::from_str::<Observation>(&json)?;
        prop_assert!(!parsed.elements.is_empty());
        prop_assert_eq!(parsed, observation);
        Ok(())
    })?;
    Ok(())
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
        active_profile_id: None,
    };
    assert_eq!(
        serde_json::to_value(subsystem)?,
        serde_json::json!({"status":"healthy","detail":null})
    );
    Ok(())
}

fn fixed_time() -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339("2026-05-23T00:00:00Z").map(|value| value.with_timezone(&Utc))
}

fn sample_event() -> Result<Event, chrono::ParseError> {
    Ok(Event {
        seq: 10,
        at: fixed_time()?,
        source: EventSource::PerceptionHud,
        kind: "hud-value-changed".to_owned(),
        data: serde_json::json!({
            "field": "hp",
            "old": 25,
            "new": 15,
            "confidence": 0.98
        }),
        correlations: vec![EventRef {
            seq: 9,
            relation: "supersedes".to_owned(),
        }],
    })
}

#[allow(clippy::too_many_lines)]
fn sample_observation() -> Result<Observation, chrono::ParseError> {
    let at = fixed_time()?;
    let focused_id = element_id(0x12ab, "0a1b2c3d");
    let parent_id = element_id(0x12ab, "ff");
    let event = sample_event()?;
    let mut hud = HudReadings::default();
    hud.by_name.insert(
        "hp".to_owned(),
        HudReading {
            raw_text: "15".to_owned(),
            parsed: HudValue::Number(15.0),
            confidence: 0.98,
            stale_ms: 16,
        },
    );

    Ok(Observation {
        seq: 123,
        at,
        mode: PerceptionMode::Hybrid,
        foreground: ForegroundContext {
            hwnd: 0x12ab,
            pid: 4242,
            process_name: "notepad.exe".to_owned(),
            process_path: "C:\\Windows\\System32\\notepad.exe".to_owned(),
            window_title: "Untitled - Notepad".to_owned(),
            window_bounds: Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.25,
            profile_id: Some("windows.notepad".to_owned()),
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        },
        focused: Some(FocusedElement {
            element_id: focused_id.clone(),
            name: "Text Editor".to_owned(),
            role: "Edit".to_owned(),
            automation_id: Some("15".to_owned()),
            bbox: Rect {
                x: 10,
                y: 20,
                w: 600,
                h: 400,
            },
            enabled: true,
            patterns: vec![UiaPattern::Value, UiaPattern::Text],
            value: Some("hello".to_owned()),
            selected_text: None,
        }),
        elements: vec![AccessibleNode {
            element_id: focused_id,
            parent: Some(parent_id),
            name: "Text Editor".to_owned(),
            role: "Edit".to_owned(),
            automation_id: Some("15".to_owned()),
            bbox: Rect {
                x: 10,
                y: 20,
                w: 600,
                h: 400,
            },
            enabled: true,
            focused: true,
            patterns: vec![UiaPattern::Value, UiaPattern::Text],
            children_count: 0,
            depth: 1,
        }],
        entities: vec![DetectedEntity {
            entity_id: entity_id(7),
            track_id: 7,
            class_label: "cursor".to_owned(),
            bbox: Rect {
                x: 120,
                y: 130,
                w: 16,
                h: 24,
            },
            confidence: 0.75,
            first_seen_at: at,
            last_seen_at: at,
            velocity_px_per_s: Some((0.0, 0.0)),
        }],
        hud,
        audio: AudioContext {
            rms_db: -45.0,
            vad_speech_recent: false,
            recent_events: Vec::new(),
            direction_estimate: None,
        },
        recent_events: vec![event.summary()],
        clipboard_summary: Some(ClipboardSummary {
            formats: vec!["text/plain".to_owned()],
            text_len: Some(5),
            text_excerpt: Some("hello".to_owned()),
            redacted: false,
        }),
        fs_recent: vec![FsEvent {
            at,
            path: "C:\\Users\\owner\\note.txt".to_owned(),
            kind: FsEventKind::Modified,
            size_bytes: Some(5),
        }],
        diagnostics: ObservationDiagnostics {
            assembled_in_ms: 4.5,
            sensor_latency_ms: BTreeMap::new(),
            a11y_enabled: true,
            pixel_enabled: true,
            audio_enabled: false,
            a11y_status: SensorStatus::Healthy,
            capture_status: SensorStatus::Healthy,
            detection_status: SensorStatus::DegradedSensorFailed {
                reason_code: "DETECTION_MODEL_NOT_LOADED".to_owned(),
            },
            audio_status: SensorStatus::Disabled,
            elements_truncated: false,
            entities_truncated: false,
            size_bytes: 2048,
            size_estimate_tokens: 512,
        },
    })
}

#[allow(clippy::too_many_lines)]
fn observation_strategy() -> impl Strategy<Value = Observation> {
    (
        1_u64..1_000_000,
        "[a-z]{1,8}",
        "[a-z]{1,12}",
        0_u32..20_000,
        1_u64..10_000,
        0.0_f32..1.0,
    )
        .prop_map(|(seq, process, role, pid, track_id, confidence)| {
            let seconds = 1_700_000_000 + (seq % 86_400);
            let at = DateTime::<Utc>::from(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
            );
            let element = element_id(i64::from(pid), "aa");
            Observation {
                seq,
                at,
                mode: PerceptionMode::A11yOnly,
                foreground: ForegroundContext {
                    hwnd: i64::from(pid),
                    pid,
                    process_name: format!("{process}.exe"),
                    process_path: format!("C:\\Apps\\{process}.exe"),
                    window_title: process,
                    window_bounds: Rect {
                        x: 0,
                        y: 0,
                        w: 640,
                        h: 480,
                    },
                    monitor_index: 0,
                    dpi_scale: 1.0,
                    profile_id: None,
                    steam_appid: None,
                    is_fullscreen: false,
                    is_dwm_composed: true,
                },
                focused: Some(FocusedElement {
                    element_id: element.clone(),
                    name: "Focused".to_owned(),
                    role: role.clone(),
                    automation_id: None,
                    bbox: Rect {
                        x: 1,
                        y: 2,
                        w: 3,
                        h: 4,
                    },
                    enabled: true,
                    patterns: vec![UiaPattern::Invoke],
                    value: None,
                    selected_text: None,
                }),
                elements: vec![AccessibleNode {
                    element_id: element,
                    parent: None,
                    name: "Focused".to_owned(),
                    role,
                    automation_id: None,
                    bbox: Rect {
                        x: 1,
                        y: 2,
                        w: 3,
                        h: 4,
                    },
                    enabled: true,
                    focused: true,
                    patterns: vec![UiaPattern::Invoke],
                    children_count: 0,
                    depth: 0,
                }],
                entities: vec![DetectedEntity {
                    entity_id: entity_id(track_id),
                    track_id,
                    class_label: "target".to_owned(),
                    bbox: Rect {
                        x: 5,
                        y: 6,
                        w: 7,
                        h: 8,
                    },
                    confidence,
                    first_seen_at: at,
                    last_seen_at: at,
                    velocity_px_per_s: None,
                }],
                hud: HudReadings::default(),
                audio: AudioContext::default(),
                recent_events: Vec::new(),
                clipboard_summary: None,
                fs_recent: Vec::new(),
                diagnostics: ObservationDiagnostics {
                    assembled_in_ms: 1.0,
                    sensor_latency_ms: BTreeMap::new(),
                    a11y_enabled: true,
                    pixel_enabled: false,
                    audio_enabled: false,
                    a11y_status: SensorStatus::Healthy,
                    capture_status: SensorStatus::Disabled,
                    detection_status: SensorStatus::Disabled,
                    audio_status: SensorStatus::Disabled,
                    elements_truncated: false,
                    entities_truncated: false,
                    size_bytes: 512,
                    size_estimate_tokens: 128,
                },
            }
        })
}
