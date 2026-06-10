use serde::{Deserialize, Serialize};
use synapse_core::Rect;

use crate::{PerceptionError, PerceptionResult};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextRegion {
    pub text: String,
    pub bbox: Rect,
    pub confidence: f32,
}

pub trait OcrProvider {
    /// Reads text from a screen-coordinate region.
    ///
    /// # Errors
    ///
    /// Returns a structured perception error when OCR cannot run or finds no text.
    fn read_text(&self, region: Rect) -> PerceptionResult<Vec<TextRegion>>;
}

#[derive(Copy, Clone, Debug, Default)]
pub struct SystemOcrProvider;

impl OcrProvider for SystemOcrProvider {
    fn read_text(&self, region: Rect) -> PerceptionResult<Vec<TextRegion>> {
        read_text(region)
    }
}

/// Reads OCR text from a screen-coordinate region.
///
/// # Errors
///
/// Returns `OCR_NO_TEXT` for an empty region and `OCR_BACKEND_UNAVAILABLE`
/// when the platform OCR backend cannot run.
pub fn read_text(region: Rect) -> PerceptionResult<Vec<TextRegion>> {
    if is_empty_region(region) {
        return Err(PerceptionError::OcrNoText { region });
    }
    platform::read_text(region)
}

/// Reads OCR text with an injected provider.
///
/// # Errors
///
/// Returns `OCR_NO_TEXT` for invalid/empty regions or empty provider output.
pub fn read_text_with_provider(
    provider: &dyn OcrProvider,
    region: Rect,
) -> PerceptionResult<Vec<TextRegion>> {
    if is_empty_region(region) {
        return Err(PerceptionError::OcrNoText { region });
    }
    let words = provider.read_text(region)?;
    if words.is_empty() {
        return Err(PerceptionError::OcrNoText { region });
    }
    Ok(words)
}

#[must_use]
pub const fn is_empty_region(region: Rect) -> bool {
    region.w <= 0 || region.h <= 0
}

#[cfg(windows)]
/// Runs `WinRT` OCR over a caller-provided `SoftwareBitmap`.
///
/// # Errors
///
/// Returns `OCR_BACKEND_UNAVAILABLE` when `WinRT` cannot initialize or rejects
/// the bitmap, and `OCR_NO_TEXT` when OCR completes with no recognized words.
pub fn read_text_from_software_bitmap(
    region: Rect,
    bitmap: &windows::Graphics::Imaging::SoftwareBitmap,
) -> PerceptionResult<Vec<TextRegion>> {
    if is_empty_region(region) {
        return Err(PerceptionError::OcrNoText { region });
    }
    platform::read_text_from_software_bitmap(region, bitmap)
}

#[cfg(windows)]
/// Runs `WinRT` OCR over caller-provided BGRA screen-region bytes.
///
/// # Errors
///
/// Returns `OCR_BACKEND_UNAVAILABLE` when the bitmap dimensions or byte length
/// are invalid or `WinRT` cannot run, and `OCR_NO_TEXT` when OCR completes with
/// no recognized words.
pub fn read_text_from_bgra_bitmap(
    region: Rect,
    width: u32,
    height: u32,
    bytes: &[u8],
) -> PerceptionResult<Vec<TextRegion>> {
    if is_empty_region(region) {
        return Err(PerceptionError::OcrNoText { region });
    }
    platform::read_text_from_bgra_bitmap(region, width, height, bytes)
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::{
        path::{Path, PathBuf},
        process::Command,
    };

    use serde::Deserialize;
    use synapse_core::Rect;

    use super::{PerceptionError, PerceptionResult, TextRegion};

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct HostOcrWord {
        text: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        confidence: f32,
    }

    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    enum HostOcrPayload {
        Words(Vec<HostOcrWord>),
        Word(HostOcrWord),
    }

    const POWERSHELL_CANDIDATES: &[&str] = &[
        "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe",
        "/mnt/c/Program Files/PowerShell/7/pwsh.exe",
        "powershell.exe",
        "pwsh.exe",
    ];

    pub fn read_text(region: Rect) -> PerceptionResult<Vec<TextRegion>> {
        let powershell = powershell_path()?;
        let script = host_ocr_script(region);
        let output = Command::new(&powershell)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"])
            .arg(script)
            .output()
            .map_err(|err| backend_unavailable(format!("host OCR command failed: {err}")))?;

        if !output.status.success() {
            return Err(backend_unavailable(format!(
                "host OCR exited with status {:?}: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let stdout = String::from_utf8(output.stdout)
            .map_err(|err| backend_unavailable(format!("host OCR stdout was not UTF-8: {err}")))?;
        let trimmed = stdout.trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return Err(PerceptionError::OcrNoText { region });
        }
        let payload: HostOcrPayload = serde_json::from_str(trimmed)
            .map_err(|err| backend_unavailable(format!("host OCR JSON parse failed: {err}")))?;
        let words = match payload {
            HostOcrPayload::Words(words) => words,
            HostOcrPayload::Word(word) => vec![word],
        };
        if words.is_empty() {
            return Err(PerceptionError::OcrNoText { region });
        }
        let regions = words
            .into_iter()
            .filter(|word| !word.text.trim().is_empty())
            .map(|word| TextRegion {
                text: word.text,
                bbox: Rect {
                    x: region.x.saturating_add(word.x),
                    y: region.y.saturating_add(word.y),
                    w: word.w,
                    h: word.h,
                },
                confidence: word.confidence,
            })
            .collect::<Vec<_>>();
        if regions.is_empty() {
            Err(PerceptionError::OcrNoText { region })
        } else {
            Ok(regions)
        }
    }

    fn powershell_path() -> PerceptionResult<PathBuf> {
        POWERSHELL_CANDIDATES
            .iter()
            .map(Path::new)
            .find(|path| path.exists())
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                backend_unavailable(
                    "host Windows PowerShell executable was not found under /mnt/c or PATH"
                        .to_owned(),
                )
            })
    }

    const fn backend_unavailable(detail: String) -> PerceptionError {
        PerceptionError::OcrBackendUnavailable { detail }
    }

    fn host_ocr_script(region: Rect) -> String {
        format!(
            "$x = {}\n$y = {}\n$w = {}\n$h = {}\n{}",
            region.x, region.y, region.w, region.h, HOST_OCR_SCRIPT_BODY
        )
    }

    const HOST_OCR_SCRIPT_BODY: &str = r"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Storage.Streams.IRandomAccessStreamWithContentType, Windows.Storage.Streams, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.SoftwareBitmap, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
$null = [Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime]
$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and
    $_.GetParameters().Count -eq 1 -and
    $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
})[0]
function Await($op, $type) {
    $asTask = $asTaskGeneric.MakeGenericMethod($type)
    $task = $asTask.Invoke($null, @($op))
    $task.Wait() | Out-Null
    $task.Result
}
$dir = Join-Path $env:TEMP 'synapse-ocr'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$path = Join-Path $dir ('ocr-{0}-{1}.png' -f $PID, [Guid]::NewGuid().ToString('N'))
$bitmap = $null
$graphics = $null
try {
    $bitmap = New-Object System.Drawing.Bitmap($w, $h)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($x, $y, 0, 0, $bitmap.Size)
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($path)) ([Windows.Storage.StorageFile])
    $stream = Await ($file.OpenReadAsync()) ([Windows.Storage.Streams.IRandomAccessStreamWithContentType])
    $decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
    $softwareBitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
    $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
    if ($null -eq $engine) {
        throw 'Windows.Media.Ocr returned no recognizer for the user profile languages'
    }
    $result = Await ($engine.RecognizeAsync($softwareBitmap)) ([Windows.Media.Ocr.OcrResult])
    $rows = @()
    foreach ($line in $result.Lines) {
        foreach ($word in $line.Words) {
            $rect = $word.BoundingRect
            $rows += [pscustomobject]@{
                text = [string]$word.Text
                x = [int][Math]::Round($rect.X)
                y = [int][Math]::Round($rect.Y)
                w = [int][Math]::Round($rect.Width)
                h = [int][Math]::Round($rect.Height)
                confidence = 1.0
            }
        }
    }
    if ($rows.Count -eq 0) {
        '[]'
    } else {
        $rows | ConvertTo-Json -Compress
    }
} finally {
    if ($null -ne $graphics) { $graphics.Dispose() }
    if ($null -ne $bitmap) { $bitmap.Dispose() }
    Remove-Item -LiteralPath $path -ErrorAction SilentlyContinue
}
";
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
mod platform {
    use synapse_core::Rect;

    use super::{PerceptionError, PerceptionResult, TextRegion};

    pub fn read_text(_region: Rect) -> PerceptionResult<Vec<TextRegion>> {
        Err(PerceptionError::OcrBackendUnavailable {
            detail: "OCR backend is implemented on Windows and WSL/Linux with Windows host OCR"
                .to_owned(),
        })
    }
}

#[cfg(windows)]
mod platform {
    use std::sync::OnceLock;

    use image::{ImageBuffer, Rgba, imageops::FilterType};
    use synapse_capture::screen_region_to_bgra_bitmap;
    use synapse_core::Rect;
    use windows::{
        Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::{OcrEngine, OcrResult},
        Storage::Streams::DataWriter,
    };

    use super::{PerceptionError, PerceptionResult, TextRegion};

    const OCR_MIN_RECOGNITION_HEIGHT_PX: u32 = 64;
    const OCR_MAX_UPSCALE: u32 = 6;

    pub fn read_text(region: Rect) -> PerceptionResult<Vec<TextRegion>> {
        let captured = screen_region_to_bgra_bitmap(region)
            .map_err(|err| backend_unavailable(err.to_string()))?;
        let (bitmap, scale) =
            ocr_bitmap_from_bgra(captured.width, captured.height, &captured.bytes)?;
        let engine = ocr_engine()?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|err| backend_unavailable(err.to_string()))?
            .join()
            .map_err(|err| backend_unavailable(err.to_string()))?;
        text_regions_from_result(region, &result, f64::from(scale))
    }

    pub fn read_text_from_software_bitmap(
        region: Rect,
        bitmap: &windows::Graphics::Imaging::SoftwareBitmap,
    ) -> PerceptionResult<Vec<TextRegion>> {
        let engine = ocr_engine()?;
        let result = engine
            .RecognizeAsync(bitmap)
            .map_err(|err| backend_unavailable(err.to_string()))?
            .join()
            .map_err(|err| backend_unavailable(err.to_string()))?;
        text_regions_from_result(region, &result, 1.0)
    }

    pub fn read_text_from_bgra_bitmap(
        region: Rect,
        width: u32,
        height: u32,
        bytes: &[u8],
    ) -> PerceptionResult<Vec<TextRegion>> {
        let (bitmap, scale) = ocr_bitmap_from_bgra(width, height, bytes)?;
        let engine = ocr_engine()?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|err| backend_unavailable(err.to_string()))?
            .join()
            .map_err(|err| backend_unavailable(err.to_string()))?;
        text_regions_from_result(region, &result, f64::from(scale))
    }

    fn ocr_engine() -> PerceptionResult<&'static OcrEngine> {
        static ENGINE: OnceLock<Result<OcrEngine, String>> = OnceLock::new();
        ENGINE
            .get_or_init(|| {
                OcrEngine::TryCreateFromUserProfileLanguages().map_err(|err| err.to_string())
            })
            .as_ref()
            .map_err(|detail| backend_unavailable(detail.clone()))
    }

    fn text_regions_from_result(
        region: Rect,
        result: &OcrResult,
        scale: f64,
    ) -> PerceptionResult<Vec<TextRegion>> {
        let lines = result
            .Lines()
            .map_err(|err| backend_unavailable(err.to_string()))?;
        let mut output = Vec::new();
        for line_index in 0..lines
            .Size()
            .map_err(|err| backend_unavailable(err.to_string()))?
        {
            let line = lines
                .GetAt(line_index)
                .map_err(|err| backend_unavailable(err.to_string()))?;
            let words = line
                .Words()
                .map_err(|err| backend_unavailable(err.to_string()))?;
            for word_index in 0..words
                .Size()
                .map_err(|err| backend_unavailable(err.to_string()))?
            {
                let word = words
                    .GetAt(word_index)
                    .map_err(|err| backend_unavailable(err.to_string()))?;
                let bbox = word
                    .BoundingRect()
                    .map_err(|err| backend_unavailable(err.to_string()))?;
                output.push(TextRegion {
                    text: word
                        .Text()
                        .map_err(|err| backend_unavailable(err.to_string()))?
                        .to_string_lossy(),
                    bbox: Rect {
                        x: region.x.saturating_add(round_scaled_to_i32(bbox.X, scale)),
                        y: region.y.saturating_add(round_scaled_to_i32(bbox.Y, scale)),
                        w: round_scaled_to_i32(bbox.Width, scale).max(1),
                        h: round_scaled_to_i32(bbox.Height, scale).max(1),
                    },
                    confidence: 1.0,
                });
            }
        }
        if output.is_empty() {
            Err(PerceptionError::OcrNoText { region })
        } else {
            Ok(output)
        }
    }

    fn round_scaled_to_i32(value: f32, scale: f64) -> i32 {
        let divisor = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        round_to_i32(f64::from(value) / divisor)
    }

    #[allow(clippy::cast_possible_truncation)]
    fn round_to_i32(value: f64) -> i32 {
        if !value.is_finite() {
            0
        } else if value >= f64::from(i32::MAX) {
            i32::MAX
        } else if value <= f64::from(i32::MIN) {
            i32::MIN
        } else {
            value.round() as i32
        }
    }

    const fn backend_unavailable(detail: String) -> PerceptionError {
        PerceptionError::OcrBackendUnavailable { detail }
    }

    fn ocr_bitmap_from_bgra(
        width: u32,
        height: u32,
        bytes: &[u8],
    ) -> PerceptionResult<(SoftwareBitmap, u32)> {
        validate_bgra_len(width, height, bytes)?;
        let scale = recognition_upscale(height);
        let (width, height, bytes) = if scale > 1 {
            let width = width.checked_mul(scale).ok_or_else(|| {
                backend_unavailable(format!("scaled OCR width overflowed: {width} * {scale}"))
            })?;
            let height = height.checked_mul(scale).ok_or_else(|| {
                backend_unavailable(format!("scaled OCR height overflowed: {height} * {scale}"))
            })?;
            (
                width,
                height,
                upscale_bgra(bytes, width / scale, height / scale, scale)?,
            )
        } else {
            (width, height, bytes.to_vec())
        };
        let bitmap = software_bitmap_from_bgra(&bytes, width, height)?;
        Ok((bitmap, scale))
    }

    fn validate_bgra_len(width: u32, height: u32, bytes: &[u8]) -> PerceptionResult<()> {
        if width == 0 || height == 0 {
            return Err(backend_unavailable(format!(
                "OCR BGRA bitmap dimensions must be non-zero: {width}x{height}"
            )));
        }
        let expected_len = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| backend_unavailable("OCR BGRA dimensions overflowed".to_owned()))?;
        let actual_len = u64::try_from(bytes.len())
            .map_err(|_err| backend_unavailable("OCR BGRA byte length was invalid".to_owned()))?;
        if actual_len != expected_len {
            return Err(backend_unavailable(format!(
                "OCR BGRA buffer length mismatch: expected {expected_len} bytes, got {actual_len}"
            )));
        }
        Ok(())
    }

    fn recognition_upscale(height: u32) -> u32 {
        if height == 0 || height >= OCR_MIN_RECOGNITION_HEIGHT_PX {
            1
        } else {
            OCR_MIN_RECOGNITION_HEIGHT_PX
                .div_ceil(height)
                .clamp(1, OCR_MAX_UPSCALE)
        }
    }

    fn upscale_bgra(
        bytes: &[u8],
        width: u32,
        height: u32,
        scale: u32,
    ) -> PerceptionResult<Vec<u8>> {
        let image = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bytes.to_vec())
            .ok_or_else(|| {
                backend_unavailable("captured OCR BGRA buffer size was invalid".to_owned())
            })?;
        let scaled_width = width.checked_mul(scale).ok_or_else(|| {
            backend_unavailable(format!("scaled OCR width overflowed: {width} * {scale}"))
        })?;
        let scaled_height = height.checked_mul(scale).ok_or_else(|| {
            backend_unavailable(format!("scaled OCR height overflowed: {height} * {scale}"))
        })?;
        Ok(
            image::imageops::resize(&image, scaled_width, scaled_height, FilterType::Nearest)
                .into_raw(),
        )
    }

    fn software_bitmap_from_bgra(
        bytes: &[u8],
        width: u32,
        height: u32,
    ) -> PerceptionResult<SoftwareBitmap> {
        let width = i32::try_from(width)
            .map_err(|err| backend_unavailable(format!("OCR bitmap width was invalid: {err}")))?;
        let height = i32::try_from(height)
            .map_err(|err| backend_unavailable(format!("OCR bitmap height was invalid: {err}")))?;
        let writer = DataWriter::new().map_err(|err| backend_unavailable(err.to_string()))?;
        writer
            .WriteBytes(bytes)
            .map_err(|err| backend_unavailable(err.to_string()))?;
        let buffer = writer
            .DetachBuffer()
            .map_err(|err| backend_unavailable(err.to_string()))?;
        SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            width,
            height,
            BitmapAlphaMode::Ignore,
        )
        .map_err(|err| backend_unavailable(err.to_string()))
    }

    #[cfg(test)]
    mod tests {
        use super::{OCR_MAX_UPSCALE, recognition_upscale};

        #[test]
        fn small_screen_ocr_regions_are_upscaled_before_recognition() {
            assert_eq!(recognition_upscale(16), 4);
            assert_eq!(recognition_upscale(32), 2);
            assert_eq!(recognition_upscale(64), 1);
            assert_eq!(recognition_upscale(0), 1);
            assert_eq!(recognition_upscale(1), OCR_MAX_UPSCALE);
        }
    }
}
