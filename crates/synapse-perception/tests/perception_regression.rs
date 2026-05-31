use std::{
    collections::BTreeMap,
    error::Error,
    io::{self, Write},
};

use chrono::Utc;
use synapse_core::{
    AccessibleNode, AudioContext, DetectedEntity, FocusedElement, ForegroundContext, HudReadings,
    PerceptionMode, Rect, SensorStatus, UiaPattern, element_id, entity_id, error_codes,
};
use synapse_perception::{
    A11yTreeSummary, ObservationAssembler, ObservationInput, ObserveInclude, OcrProvider,
    TextRegion, auto_mode, auto_mode_with_a11y, parse_perception_mode, read_text,
    read_text_with_provider,
};

type TestResult = Result<(), Box<dyn Error>>;

fn regression_log(args: std::fmt::Arguments<'_>) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_fmt(args)?;
    stdout.write_all(b"\n")
}

#[derive(Clone, Debug)]
struct StaticOcr {
    output: Vec<TextRegion>,
}

impl OcrProvider for StaticOcr {
    fn read_text(&self, _region: Rect) -> synapse_perception::PerceptionResult<Vec<TextRegion>> {
        Ok(self.output.clone())
    }
}

fn notepad_input() -> ObservationInput {
    let at = Utc::now();
    let focused_id = element_id(0x1234, "0000002a00001234");
    let elements = vec![
        node(0, "Notepad", "Window", false),
        node(1, "Document", "Edit", true),
        node(1, "File", "MenuItem", false),
        node(1, "Edit", "MenuItem", false),
        node(1, "View", "MenuItem", false),
        node(1, "Status", "Text", false),
    ];
    let mut latency = BTreeMap::new();
    latency.insert("a11y".to_owned(), 1.25);
    latency.insert("capture".to_owned(), 0.50);
    latency.insert("detection".to_owned(), 0.0);
    latency.insert("audio".to_owned(), 0.0);
    ObservationInput {
        foreground: ForegroundContext {
            hwnd: 0x1234,
            pid: 44,
            process_name: "notepad.exe".to_owned(),
            process_path: "C:\\Windows\\System32\\notepad.exe".to_owned(),
            window_title: "manual-regression.txt - Notepad".to_owned(),
            window_bounds: Rect {
                x: 10,
                y: 20,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        },
        focused: Some(FocusedElement {
            element_id: focused_id,
            name: "Document".to_owned(),
            role: "Edit".to_owned(),
            automation_id: Some("15".to_owned()),
            bbox: Rect {
                x: 12,
                y: 80,
                w: 760,
                h: 480,
            },
            enabled: true,
            patterns: vec![UiaPattern::Text, UiaPattern::Value],
            value: Some("synthetic notepad text".to_owned()),
            selected_text: None,
        }),
        elements,
        entities: vec![DetectedEntity {
            entity_id: entity_id(9),
            track_id: 9,
            class_label: "cursor".to_owned(),
            bbox: Rect {
                x: 40,
                y: 90,
                w: 8,
                h: 20,
            },
            confidence: 0.80,
            first_seen_at: at,
            last_seen_at: at,
            velocity_px_per_s: None,
        }],
        hud: HudReadings::default(),
        audio: AudioContext::default(),
        recent_events: Vec::new(),
        clipboard_summary: None,
        fs_recent: Vec::new(),
        sensor_latency_ms: latency,
        a11y_status: SensorStatus::Healthy,
        capture_status: SensorStatus::Healthy,
        detection_status: SensorStatus::Disabled,
        audio_status: SensorStatus::Disabled,
        mode_override: None,
    }
}

fn node(depth: u32, name: &str, role: &str, focused: bool) -> AccessibleNode {
    let depth_i32 = i32::try_from(depth).unwrap_or(0);
    AccessibleNode {
        element_id: element_id(0x1234, &format!("0000002a{depth:08x}")),
        parent: (depth > 0).then(|| element_id(0x1234, "0000002a00000000")),
        name: name.to_owned(),
        role: role.to_owned(),
        automation_id: None,
        value: None,
        bbox: Rect {
            x: 10 + depth_i32,
            y: 20 + depth_i32,
            w: 100,
            h: 30,
        },
        enabled: true,
        focused,
        patterns: Vec::new(),
        children_count: 0,
        depth,
    }
}

#[test]
fn assemble_default_notepad_under_6kb_with_latency_readback() -> TestResult {
    let input = notepad_input();
    regression_log(format_args!(
        "regression_check=observation edge=notepad_default before=process:{} nodes:{}",
        input.foreground.process_name,
        input.elements.len()
    ))?;
    let observation = ObservationAssembler::new().assemble(ObserveInclude::default(), input)?;
    let bytes = serde_json::to_vec(&observation)?;
    regression_log(format_args!(
        "regression_check=observation edge=notepad_default after=bytes:{} mode:{:?} process:{} focused_role:{} a11y_latency:{:?}",
        bytes.len(),
        observation.mode,
        observation.foreground.process_name,
        observation
            .focused
            .as_ref()
            .map_or("", |item| item.role.as_str()),
        observation.diagnostics.sensor_latency_ms.get("a11y")
    ))?;
    assert!(bytes.len() <= 6 * 1024);
    assert_eq!(observation.mode, PerceptionMode::A11yOnly);
    assert_eq!(observation.foreground.process_name, "notepad.exe");
    assert_eq!(
        observation.focused.as_ref().map(|item| item.role.as_str()),
        Some("Edit")
    );
    assert!(
        observation
            .diagnostics
            .sensor_latency_ms
            .contains_key("capture")
    );
    assert_eq!(
        observation.diagnostics.size_bytes,
        u32::try_from(bytes.len())?
    );
    Ok(())
}

#[test]
fn include_flags_are_independently_testable() -> TestResult {
    let assembler = ObservationAssembler::new();
    let no_subtree = assembler.assemble(
        ObserveInclude {
            elements: false,
            ..ObserveInclude::default()
        },
        notepad_input(),
    )?;
    regression_log(format_args!(
        "regression_check=observe_include edge=no_subtree after=focused:{} elements:{} truncated:{}",
        no_subtree.focused.is_some(),
        no_subtree.elements.len(),
        no_subtree.diagnostics.elements_truncated
    ))?;
    assert!(no_subtree.focused.is_some());
    assert!(no_subtree.elements.is_empty());
    assert!(no_subtree.diagnostics.elements_truncated);

    let no_focus = assembler.assemble(
        ObserveInclude {
            focused: false,
            ..ObserveInclude::default()
        },
        notepad_input(),
    )?;
    regression_log(format_args!(
        "regression_check=observe_include edge=no_focused after=focused:{} elements:{}",
        no_focus.focused.is_some(),
        no_focus.elements.len()
    ))?;
    assert!(no_focus.focused.is_none());
    assert!(!no_focus.elements.is_empty());

    let limited_detection = assembler.assemble(
        ObserveInclude {
            entities: true,
            max_entities: 0,
            ..ObserveInclude::default()
        },
        notepad_input(),
    )?;
    regression_log(format_args!(
        "regression_check=observe_include edge=detections_limited after=entities:{} truncated:{}",
        limited_detection.entities.len(),
        limited_detection.diagnostics.entities_truncated
    ))?;
    assert!(limited_detection.entities.is_empty());
    assert!(limited_detection.diagnostics.entities_truncated);
    Ok(())
}

#[test]
fn assembler_all_sensors_unavailable_fails_closed() -> TestResult {
    let input = ObservationInput::new(notepad_input().foreground);
    regression_log(format_args!(
        "regression_check=observation edge=no_sensors before=a11y:{:?} capture:{:?} detection:{:?} audio:{:?}",
        input.a11y_status, input.capture_status, input.detection_status, input.audio_status
    ))?;
    let after = ObservationAssembler::new().assemble(ObserveInclude::default(), input);
    regression_log(format_args!(
        "regression_check=observation edge=no_sensors after={after:?}"
    ))?;
    assert_eq!(
        after.err().map(|err| err.code()),
        Some(error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE)
    );
    Ok(())
}

#[test]
fn auto_mode_edges_and_invalid_manual_override() -> TestResult {
    let notepad = notepad_input().foreground;
    let notepad_default = auto_mode(&notepad);
    regression_log(format_args!(
        "regression_check=perception_mode edge=notepad_default before=process:{} after={notepad_default:?}",
        notepad.process_name
    ))?;
    assert_eq!(notepad_default, PerceptionMode::A11yOnly);

    let notepad_mode = auto_mode_with_a11y(
        &notepad,
        &A11yTreeSummary {
            node_count: 6,
            max_depth: 1,
        },
    );
    regression_log(format_args!(
        "regression_check=perception_mode edge=notepad_rich_a11y after={notepad_mode:?}"
    ))?;
    assert_eq!(notepad_mode, PerceptionMode::A11yOnly);

    let mut game = notepad.clone();
    game.process_name = "starfield.exe".to_owned();
    let game_default = auto_mode(&game);
    regression_log(format_args!(
        "regression_check=perception_mode edge=game_default before=process:{} after={game_default:?}",
        game.process_name
    ))?;
    assert_eq!(game_default, PerceptionMode::Hybrid);

    let game_mode = auto_mode_with_a11y(
        &game,
        &A11yTreeSummary {
            node_count: 20,
            max_depth: 3,
        },
    );
    regression_log(format_args!(
        "regression_check=perception_mode edge=game_rich_a11y after={game_mode:?}"
    ))?;
    assert_eq!(game_mode, PerceptionMode::Hybrid);

    let sparse_mode = auto_mode_with_a11y(
        &notepad,
        &A11yTreeSummary {
            node_count: 1,
            max_depth: 0,
        },
    );
    regression_log(format_args!(
        "regression_check=perception_mode edge=sparse_a11y after={sparse_mode:?}"
    ))?;
    assert_eq!(sparse_mode, PerceptionMode::Hybrid);

    let invalid = parse_perception_mode("telepathy");
    regression_log(format_args!(
        "regression_check=perception_mode edge=invalid before=telepathy after={invalid:?}"
    ))?;
    assert_eq!(
        invalid.err().map(|err| err.code()),
        Some(error_codes::PERCEPTION_MODE_INVALID)
    );
    Ok(())
}

#[test]
fn ocr_empty_region_and_backend_edges_are_observable() -> TestResult {
    let empty = Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 64,
    };
    regression_log(format_args!(
        "regression_check=ocr edge=empty_region before=region:{empty:?}"
    ))?;
    let empty_after = read_text(empty);
    regression_log(format_args!(
        "regression_check=ocr edge=empty_region after={empty_after:?}"
    ))?;
    assert_eq!(
        empty_after.err().map(|err| err.code()),
        Some(error_codes::OCR_NO_TEXT)
    );

    #[cfg(not(windows))]
    {
        let region = Rect {
            x: 0,
            y: 0,
            w: 256,
            h: 64,
        };
        regression_log(format_args!(
            "regression_check=ocr edge=backend before=region:{region:?}"
        ))?;
        let backend_after = read_text(region);
        regression_log(format_args!(
            "regression_check=ocr edge=backend after={backend_after:?}"
        ))?;
        match backend_after {
            Ok(words) => {
                assert!(
                    !words.is_empty(),
                    "available platform OCR must return at least one word"
                );
                regression_log(format_args!(
                    "regression_check=ocr edge=backend live_words={}",
                    words.len()
                ))?;
            }
            Err(err) => {
                assert!(
                    matches!(
                        err.code(),
                        error_codes::OCR_BACKEND_UNAVAILABLE | error_codes::OCR_NO_TEXT
                    ),
                    "platform OCR must fail closed with stable OCR code, got {err:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn ocr_provider_happy_path_and_empty_result_are_observable() -> TestResult {
    let region = Rect {
        x: 5,
        y: 7,
        w: 256,
        h: 64,
    };
    let provider = StaticOcr {
        output: vec![TextRegion {
            text: "Synapse".to_owned(),
            bbox: Rect {
                x: 9,
                y: 11,
                w: 72,
                h: 18,
            },
            confidence: 0.99,
        }],
    };
    regression_log(format_args!(
        "regression_check=ocr edge=synthetic_text before=region:{region:?} provider_words:{}",
        provider.output.len()
    ))?;
    let words = read_text_with_provider(&provider, region)?;
    let first = words
        .first()
        .ok_or_else(|| io::Error::other("missing OCR first word"))?;
    regression_log(format_args!(
        "regression_check=ocr edge=synthetic_text after=count:{} first:{} bbox:{:?}",
        words.len(),
        first.text,
        first.bbox
    ))?;
    assert_eq!(words.len(), 1);
    assert_eq!(first.text, "Synapse");
    assert_eq!(
        first.bbox,
        Rect {
            x: 9,
            y: 11,
            w: 72,
            h: 18
        }
    );

    let empty_provider = StaticOcr { output: Vec::new() };
    regression_log(format_args!(
        "regression_check=ocr edge=synthetic_empty before=region:{region:?} provider_words:{}",
        empty_provider.output.len()
    ))?;
    let empty = read_text_with_provider(&empty_provider, region);
    regression_log(format_args!(
        "regression_check=ocr edge=synthetic_empty after={empty:?}"
    ))?;
    assert_eq!(
        empty.err().map(|err| err.code()),
        Some(error_codes::OCR_NO_TEXT)
    );
    Ok(())
}

#[cfg(windows)]
#[test]
fn winrt_blank_bitmap_returns_no_text_or_backend_unavailable() -> TestResult {
    use synapse_perception::read_text_from_software_bitmap;
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};

    let region = Rect {
        x: 0,
        y: 0,
        w: 256,
        h: 64,
    };
    regression_log(format_args!(
        "regression_check=ocr edge=winrt_blank before=region:{region:?} bitmap=256x64"
    ))?;
    let Ok(bitmap) = SoftwareBitmap::Create(BitmapPixelFormat::Bgra8, region.w, region.h) else {
        regression_log(format_args!(
            "regression_check=ocr edge=winrt_blank after=backend_unavailable:software_bitmap_activation"
        ))?;
        return Ok(());
    };
    let after = read_text_from_software_bitmap(region, &bitmap);
    regression_log(format_args!(
        "regression_check=ocr edge=winrt_blank after={after:?}"
    ))?;
    let code = after.err().map(|err| err.code());
    assert!(matches!(
        code,
        Some(error_codes::OCR_NO_TEXT | error_codes::OCR_BACKEND_UNAVAILABLE)
    ));
    Ok(())
}

#[cfg(windows)]
#[test]
#[ignore = "requires an interactive Windows desktop with WinRT OCR"]
fn winrt_text_window_region_read_text_native() -> TestResult {
    use std::{
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let title = format!("synapse-ocr-regression-{stamp}");
    let expected = "Synapse OCR Regression 2468";
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         Add-Type -AssemblyName System.Drawing; \
         $form = New-Object System.Windows.Forms.Form; \
         $form.Text = '{title}'; \
         $form.Width = 900; $form.Height = 260; \
         $form.StartPosition = 'CenterScreen'; $form.TopMost = $true; \
         $label = New-Object System.Windows.Forms.Label; \
         $label.Text = '{expected}'; \
         $label.Font = New-Object System.Drawing.Font('Segoe UI', 36); \
         $label.Dock = 'Fill'; $label.TextAlign = 'MiddleCenter'; \
         $form.Controls.Add($label); \
         $form.Add_Shown({{ $form.Activate() }}); \
         [void]$form.ShowDialog();"
    );
    regression_log(format_args!(
        "regression_check=ocr edge=winrt_text_window before=title:{title} text={expected}"
    ))?;
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-STA", "-Command", &script])
        .spawn()?;

    let result = (|| -> TestResult {
        let hwnd = wait_for_window_hwnd(child.id())?;
        regression_log(format_args!(
            "regression_check=ocr edge=winrt_text_window hwnd_readback={hwnd}"
        ))?;
        let root = synapse_a11y::window_from_hwnd(hwnd)?;
        let tree = synapse_a11y::snapshot(&root, 2)?;
        let target = tree
            .nodes
            .iter()
            .find(|node| node.name.contains("Synapse") && node.bbox.w > 0 && node.bbox.h > 0)
            .or_else(|| {
                tree.nodes
                    .iter()
                    .find(|node| node.focused && node.bbox.w > 0 && node.bbox.h > 0)
            })
            .or_else(|| {
                tree.nodes
                    .iter()
                    .find(|node| node.bbox.w > 0 && node.bbox.h > 0)
            })
            .ok_or_else(|| io::Error::other("no visible text-window node"))?;
        regression_log(format_args!(
            "regression_check=ocr edge=winrt_text_window focus_readback=root:{} nodes:{} role:{} name:{} bbox:{:?}",
            tree.root,
            tree.nodes.len(),
            target.role,
            target.name,
            target.bbox
        ))?;
        let words = read_text(target.bbox)?;
        let text = words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        regression_log(format_args!(
            "regression_check=ocr edge=winrt_text_window after=text:{text} words:{}",
            words.len()
        ))?;
        assert!(text.contains("Synapse"), "actual OCR text: {text}");
        Ok(())
    })();

    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(windows)]
#[test]
#[ignore = "requires an interactive Windows desktop with WinRT OCR"]
fn winrt_ocr_capture_vs_recognize_timing_native() -> TestResult {
    use std::time::Instant;

    use synapse_capture::screen_region_to_software_bitmap;
    use synapse_perception::read_text_from_software_bitmap;

    for (name, region) in [
        (
            "small_256x64",
            Rect {
                x: 0,
                y: 0,
                w: 256,
                h: 64,
            },
        ),
        (
            "full_1080p",
            Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
        ),
    ] {
        let mut capture_ms = Vec::new();
        let mut recognize_ms = Vec::new();
        regression_log(format_args!(
            "regression_check=ocr_timing edge={name} before=region:{region:?} samples:10"
        ))?;
        for _ in 0..10 {
            let capture_started = Instant::now();
            let captured = screen_region_to_software_bitmap(region)?;
            capture_ms.push(capture_started.elapsed().as_secs_f64() * 1_000.0);

            let recognize_started = Instant::now();
            let result = read_text_from_software_bitmap(captured.region, &captured.bitmap);
            recognize_ms.push(recognize_started.elapsed().as_secs_f64() * 1_000.0);
            if matches!(
                result
                    .as_ref()
                    .err()
                    .map(synapse_perception::PerceptionError::code),
                Some(error_codes::OCR_BACKEND_UNAVAILABLE)
            ) {
                return Err(io::Error::other("WinRT OCR backend unavailable").into());
            }
        }
        regression_log(format_args!(
            "regression_check=ocr_timing edge={name} after=capture_p99_ms:{:.3} recognize_p99_ms:{:.3}",
            p99_ms(capture_ms),
            p99_ms(recognize_ms)
        ))?;
    }
    Ok(())
}

#[cfg(windows)]
fn wait_for_window_hwnd(pid: u32) -> Result<i64, Box<dyn Error>> {
    for _ in 0..20 {
        match synapse_a11y::window_for_process(pid) {
            Ok(root) => {
                let tree = synapse_a11y::snapshot(&root, 0)?;
                return Ok(tree.root.parts()?.hwnd);
            }
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
    Err(io::Error::other(format!("window for process id {pid} was not visible")).into())
}

#[cfg(windows)]
fn p99_ms(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples.last().copied().unwrap_or_default()
}
