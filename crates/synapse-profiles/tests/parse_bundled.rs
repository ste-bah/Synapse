use std::{fs, time::UNIX_EPOCH};

use synapse_core::{Backend, error_codes};
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
    assert_eq!(ids, ["chrome", "notepad", "terminal", "vscode"]);
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
