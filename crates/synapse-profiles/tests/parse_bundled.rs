use std::{fs, time::UNIX_EPOCH};

use synapse_core::{Backend, HudExtractor, HudRegion, ProfileUseScope, WindowEdge, error_codes};
use synapse_profiles::{
    ProfileError, ScreenBounds, bundled_profiles_dir, parse_profile_bytes, parse_profile_file,
};

#[test]
fn bundled_profiles_parse_and_keep_natural_defaults() -> Result<(), Box<dyn std::error::Error>> {
    let dir = bundled_profiles_dir();
    let mut ids = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("toml") {
            continue;
        }
        let loaded = parse_profile_file(&path)?;
        assert_eq!(loaded.schema_version, 1, "{}", path.display());
        assert_eq!(
            loaded.defaults.mouse_curve_default,
            "natural",
            "{}",
            path.display()
        );
        assert_eq!(
            loaded.defaults.keyboard_dynamics_default,
            "natural",
            "{}",
            path.display()
        );
        ids.push(loaded.profile.id);
    }
    ids.sort();
    assert_eq!(
        ids,
        [
            "chrome",
            "luanti.minetest",
            "minecraft.java",
            "notepad",
            "terminal",
            "vscode",
        ]
    );
    Ok(())
}

#[test]
fn bundled_minecraft_profile_carries_first_game_contract() -> Result<(), Box<dyn std::error::Error>>
{
    let path = bundled_profiles_dir().join("minecraft.java.toml");
    let loaded = parse_profile_file(&path)?;
    let profile = loaded.profile;

    assert_eq!(profile.id, "minecraft.java");
    assert_eq!(profile.use_scope, ProfileUseScope::SinglePlayer);
    assert_eq!(profile.matches.len(), 2);
    assert_eq!(
        profile.detection.model_id.as_deref(),
        Some("rtdetr_v2_s_coco_onnx")
    );
    assert_eq!(
        profile.detection.classes_of_interest,
        ["player", "zombie", "skeleton", "creeper", "villager"]
    );
    assert_eq!(profile.hud.len(), 3);
    assert_eq!(profile.keymap["attack"], "lmb");
    assert_eq!(profile.keymap["place"], "rmb");
    assert_eq!(profile.keymap["sneak"], "lshift");
    assert!(
        profile
            .event_extensions
            .iter()
            .any(|extension| extension.name == "creeper_nearby")
    );
    assert_eq!(
        profile.metadata["supported_use.remote_server_allowed"],
        "false"
    );
    assert_eq!(
        profile.metadata["runtime.minecraft.configured_host_status"],
        "launcher_installed_sign_in_required_java_runtime_not_verified"
    );
    Ok(())
}

#[test]
fn parser_rejects_synthetic_invalid_profiles_with_exact_codes() {
    let cases = [
        (
            "missing_matches.toml",
            r#"
id = "missing_matches"
label = "Missing Matches"
schema_version = 1
use_scope = "productivity"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"
"#,
            error_codes::PROFILE_PARSE_ERROR,
        ),
        (
            "bad_regex.toml",
            r#"
id = "bad_regex"
label = "Bad Regex"
schema_version = 1
use_scope = "productivity"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
title_regex = "("
"#,
            error_codes::PROFILE_PARSE_ERROR,
        ),
        (
            "future_schema.toml",
            r#"
id = "future_schema"
label = "Future Schema"
schema_version = 999
use_scope = "productivity"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "future.exe"
"#,
            error_codes::PROFILE_VERSION_INCOMPATIBLE,
        ),
        (
            "invalid_keymap.toml",
            r#"
id = "invalid_keymap"
label = "Invalid Keymap"
schema_version = 1
use_scope = "productivity"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "keys.exe"

[keymap]
bad = "ctrl+dragon"
"#,
            error_codes::PROFILE_KEYMAP_INVALID,
        ),
        (
            "instant_default.toml",
            r#"
id = "instant_default"
label = "Instant Default"
schema_version = 1
use_scope = "productivity"
mouse_curve_default = "instant"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "instant.exe"
"#,
            error_codes::PROFILE_PARSE_ERROR,
        ),
    ];

    for (path, toml, code) in cases {
        let result = parse_profile_bytes(
            path,
            toml.as_bytes(),
            UNIX_EPOCH,
            ScreenBounds {
                width: 1920,
                height: 1080,
            },
        );
        let Err(error) = result else {
            panic!("{path} parsed successfully but should have failed");
        };
        assert_eq!(error.code(), code, "{path}: {error}");
    }
}

#[test]
fn parser_accepts_default_backend_alias_for_action_backends() {
    let toml = r#"
id = "hardware_alias"
label = "Hardware Alias"
schema_version = 1
use_scope = "operator_owned_test"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "hardware-alias.exe"

[backends]
default_backend = "hardware"
"#;

    let loaded = parse_profile_bytes(
        "hardware_alias.toml",
        toml.as_bytes(),
        UNIX_EPOCH,
        ScreenBounds {
            width: 1920,
            height: 1080,
        },
    )
    .unwrap_or_else(|error| panic!("default_backend alias should parse: {error}"));

    assert_eq!(loaded.profile.backends.default, Backend::Hardware);
    assert_eq!(loaded.profile.backends.keyboard_default, Backend::Auto);
    assert_eq!(loaded.profile.backends.mouse_default, Backend::Auto);
    assert_eq!(loaded.profile.backends.pad_default, Backend::Auto);
    println!(
        "readback=profile_backend_alias edge=default_backend before=default_backend after={:?} profile_id={}",
        loaded.profile.backends, loaded.profile.id
    );
}

#[test]
fn parser_accepts_mouse_button_keymap_aliases_and_metadata() {
    let toml = r#"
id = "mouse_aliases"
label = "Mouse Aliases"
schema_version = 1
use_scope = "operator_owned_test"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "luanti.exe"

[keymap]
dig = "lmb"
place = "rmb"
pan = "mmb"

[metadata]
benchmark_id = "luanti.minetest"
"supported_use.local_world_only" = "true"
"#;

    let loaded = parse_profile_bytes(
        "mouse_aliases.toml",
        toml.as_bytes(),
        UNIX_EPOCH,
        ScreenBounds {
            width: 1920,
            height: 1080,
        },
    )
    .unwrap_or_else(|error| panic!("mouse button keymap aliases should parse: {error}"));

    assert_eq!(loaded.profile.keymap["dig"], "lmb");
    assert_eq!(loaded.profile.metadata["benchmark_id"], "luanti.minetest");
    assert_eq!(
        loaded.profile.metadata["supported_use.local_world_only"],
        "true"
    );
}

#[test]
fn parser_accepts_nested_hud_fields_and_event_extensions() {
    let toml = r#"
id = "luanti_nested"
label = "Luanti Nested"
schema_version = 1
use_scope = "operator_owned_test"
mode = "pixel_only"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "luanti.exe"

[[hud]]
name = "luanti.crosshair_contrast"
extractor = { kind = "color_ratio", sample_points = [[0, 0], [24, 24]], mapping = "luma_stddev_0_1" }
parser = { kind = "number" }
region = { kind = "fraction_of_window", x = 0.48, y = 0.48, w = 0.04, h = 0.04 }

[[event_extensions]]
name = "luanti_hud_observed"
from_filter = { op = "and", args = [
    { op = "source", source = "perception_hud" },
    { op = "kind", kind = "perception.hud_observed" }
] }
emits_kind = "benchmark.luanti.hud_observed"

[[event_extensions]]
name = "luanti_entity_observed"
from_filter = { op = "and", args = [
    { op = "source", source = "perception_detection" },
    { op = "kind", kind = "entity_appeared" },
    { op = "data", path = "/profile_id", predicate = { op = "eq", value = "luanti.minetest" } }
] }
emits_kind = "benchmark.luanti.entity_observed"

[[event_extensions]]
name = "luanti_action_observed"
from_filter = { op = "and", args = [
    { op = "source", source = "action_emitter" },
    { op = "kind", kind = "action.dispatched" },
    { op = "data", path = "/tool", predicate = { op = "exists" } }
] }
emits_kind = "benchmark.luanti.action_observed"
"#;

    let loaded = parse_profile_bytes(
        "luanti_nested.toml",
        toml.as_bytes(),
        UNIX_EPOCH,
        ScreenBounds {
            width: 1920,
            height: 1080,
        },
    )
    .unwrap_or_else(|error| panic!("nested HUD/event profile should parse: {error}"));

    assert_eq!(loaded.profile.hud.len(), 1);
    assert_eq!(loaded.profile.hud[0].name, "luanti.crosshair_contrast");
    assert!(matches!(
        loaded.profile.hud[0].region,
        HudRegion::FractionOfWindow { .. }
    ));
    assert!(matches!(
        loaded.profile.hud[0].extractor,
        HudExtractor::ColorRatio { .. }
    ));
    assert_eq!(loaded.profile.event_extensions.len(), 3);
    assert_eq!(
        loaded.profile.event_extensions[0].emits_kind,
        "benchmark.luanti.hud_observed"
    );
    assert_eq!(
        loaded.profile.event_extensions[1].emits_kind,
        "benchmark.luanti.entity_observed"
    );
    assert_eq!(
        loaded.profile.event_extensions[2].emits_kind,
        "benchmark.luanti.action_observed"
    );
}

#[test]
fn parser_accepts_center_anchored_hud_region() {
    let toml = r#"
id = "center_hud"
label = "Center HUD"
schema_version = 1
use_scope = "operator_owned_test"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "center-hud.exe"

[[hud]]
name = "center.prompt"
extractor = { kind = "winrt_ocr" }
parser = { kind = "number" }
region = { kind = "anchored_to_edge", edge = "center", x_offset = -40, y_offset = -12, w = 80, h = 24 }
"#;

    println!(
        "readback=profile_hud_anchor edge=center_parse before=toml_edge:center offsets=(-40,-12) size=(80,24)"
    );
    let loaded = parse_profile_bytes(
        "center_hud.toml",
        toml.as_bytes(),
        UNIX_EPOCH,
        ScreenBounds {
            width: 1920,
            height: 1080,
        },
    )
    .unwrap_or_else(|error| panic!("center anchored HUD region should parse: {error}"));
    println!(
        "readback=profile_hud_anchor edge=center_parse after=region:{:?}",
        loaded.profile.hud[0].region
    );

    assert!(matches!(
        loaded.profile.hud[0].region,
        HudRegion::AnchoredToEdge {
            edge: WindowEdge::Center,
            x_offset: -40,
            y_offset: -12,
            w: 80,
            h: 24,
        }
    ));
}

#[test]
fn parser_rejects_trivially_true_event_extension_filter() {
    let toml = r#"
id = "always_true_event"
label = "Always True Event"
schema_version = 1
use_scope = "operator_owned_test"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "always-true.exe"

[[event_extensions]]
name = "always_true"
from_filter = { op = "all" }
emits_kind = "always.true"
"#;

    let result = parse_profile_bytes(
        "always_true_event.toml",
        toml.as_bytes(),
        UNIX_EPOCH,
        ScreenBounds {
            width: 1920,
            height: 1080,
        },
    );
    let Err(error) = result else {
        panic!("always-true event extension parsed successfully but should fail");
    };
    println!(
        "readback=profile_event_extension edge=always_true after=code:{} error:{}",
        error.code(),
        error
    );
    assert_eq!(error.code(), error_codes::PROFILE_PARSE_ERROR);
    assert!(
        error
            .to_string()
            .contains("from_filter must not be trivially always true")
    );
}

#[test]
fn parser_rejects_hud_regions_outside_screen_bounds() {
    let toml = r#"
id = "bad_hud"
label = "Bad HUD"
schema_version = 1
use_scope = "productivity"
mouse_curve_default = "natural"
keyboard_dynamics_default = "natural"

[[matches]]
exe = "hud.exe"

[[hud]]
name = "hp"
x = 2000
y = 0
w = 100
h = 100
"#;

    let result = parse_profile_bytes(
        "bad_hud.toml",
        toml.as_bytes(),
        UNIX_EPOCH,
        ScreenBounds {
            width: 1920,
            height: 1080,
        },
    );
    let Err(error) = result else {
        panic!("HUD region parsed successfully but should be outside 1920x1080 bounds");
    };
    assert!(matches!(error, ProfileError::HudRegionInvalid { .. }));
    assert_eq!(error.code(), error_codes::PROFILE_HUD_REGION_INVALID);
}
