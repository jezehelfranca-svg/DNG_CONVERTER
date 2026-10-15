use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use image::{DynamicImage, GenericImageView, ImageFormat, ImageReader, RgbImage};
use image::codecs::jpeg::JpegEncoder;

use crate::color::{apply_adaptive_grading, ColorProfile};
use crate::error::DngError;

const MAX_DIMENSION: u32 = 16384;

#[derive(Debug, Clone)]
pub struct ConversionConfig {
    pub quality: u8,
    pub color_profile: ColorProfile,
    pub auto_orient: bool,
    pub output_subfolder: String,
    pub allow_magick_fallback: bool,
}

impl Default for ConversionConfig {
    fn default() -> Self {
        Self {
            quality: 98,
            color_profile: ColorProfile::Balanced,
            auto_orient: true,
            output_subfolder: "Converted_JPEG".to_string(),
            allow_magick_fallback: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConversionResult {
    pub source_path: String,
    pub output_path: String,
    pub filename: String,
    pub output_filename: String,
    pub success: bool,
    pub source_size_bytes: u64,
    pub output_size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub duration_sec: f64,
    pub engine_used: String,
    pub error: Option<String>,
}

fn parse_tiff_orientation(bytes: &[u8]) -> u16 {
    if bytes.len() < 8 {
        return 1;
    }
    let is_le = bytes[0] == 0x49 && bytes[1] == 0x49;
    let is_be = bytes[0] == 0x4D && bytes[1] == 0x4D;
    if !is_le && !is_be {
        return 1;
    }

    let read_u16 = |offset: usize| -> Option<u16> {
        if offset + 2 > bytes.len() {
            return None;
        }
        Some(if is_le {
            u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
        } else {
            u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
        })
    };

    let read_u32 = |offset: usize| -> Option<u32> {
        if offset + 4 > bytes.len() {
            return None;
        }
        Some(if is_le {
            u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
        } else {
            u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
        })
    };

    let ifd0 = match read_u32(4) {
        Some(v) => v as usize,
        None => return 1,
    };

    let count = match read_u16(ifd0) {
        Some(v) => v as usize,
        None => return 1,
    };

    for i in 0..count {
        let entry_off = ifd0 + 2 + i * 12;
        if entry_off + 12 > bytes.len() {
            break;
        }
        if let Some(tag) = read_u16(entry_off) {
            if tag == 0x0112 {
                return read_u16(entry_off + 8).unwrap_or(1);
            }
        }
    }
    1
}

fn extract_largest_embedded_jpeg(bytes: &[u8]) -> Option<&[u8]> {
    let len = bytes.len();
    if len < 4 {
        return None;
    }

    let mut best_start = 0;
    let mut best_len = 0;
    let mut i = 0;

    while i < len - 4 {
        if bytes[i] == 0xFF && bytes[i + 1] == 0xD8 && bytes[i + 2] == 0xFF {
            let start = i;
            let mut end = None;
            let mut j = start + 4;
            while j < len - 1 {
                if bytes[j] == 0xFF && bytes[j + 1] == 0xD9 {
                    end = Some(j + 2);
                    break;
                }
                j += 1;
            }
            if let Some(e) = end {
                let stream_len = e - start;
                if stream_len > 30_000 && stream_len > best_len {
                    best_start = start;
                    best_len = stream_len;
                }
                i = e;
            } else {
                i += 3;
            }
        } else {
            i += 1;
        }
    }

    if best_len > 0 {
        Some(&bytes[best_start..best_start + best_len])
    } else {
        None
    }
}

fn apply_orientation(img: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

fn decode_dng_pure_rust(bytes: &[u8], auto_orient: bool) -> Result<(RgbImage, String), DngError> {
    let orientation = if auto_orient {
        parse_tiff_orientation(bytes)
    } else {
        1
    };

    if let Some(jpeg_bytes) = extract_largest_embedded_jpeg(bytes) {
        if let Ok(dyn_img) = image::load_from_memory_with_format(jpeg_bytes, ImageFormat::Jpeg) {
            let (w, h) = dyn_img.dimensions();
            if w <= MAX_DIMENSION && h <= MAX_DIMENSION && (w >= 1000 || h >= 1000) {
                let oriented = apply_orientation(dyn_img, orientation);
                return Ok((oriented.into_rgb8(), "Pure-Rust (Embedded Camera Preview)".to_string()));
            }
        }
    }

    if let Ok(dyn_img) = image::load_from_memory_with_format(bytes, ImageFormat::Tiff) {
        let (w, h) = dyn_img.dimensions();
        if w > MAX_DIMENSION || h > MAX_DIMENSION {
            return Err(DngError::DecompressionBomb(format!("Dimensions {}x{} exceed limit", w, h)));
        }
        let oriented = apply_orientation(dyn_img, orientation);
        return Ok((oriented.into_rgb8(), "Pure-Rust (TIFF Raster)".to_string()));
    }

    Err(DngError::UnsupportedFormat("No supported embedded stream or uncompressed TIFF raster in DNG".to_string()))
}

fn convert_with_magick_fallback(
    source_path: &Path,
    output_path: &Path,
    config: &ConversionConfig,
) -> Result<(), DngError> {
    let mut cmd = Command::new("magick");
    cmd.arg(format!("{}[0]", source_path.display()));

    if config.auto_orient {
        cmd.arg("-auto-orient");
    }

    match config.color_profile {
        ColorProfile::Balanced => {
            cmd.args(["-auto-level", "-sigmoidal-contrast", "3x50%", "-modulate", "100,112,100"]);
        }
        ColorProfile::Vibrant => {
            cmd.args(["-auto-level", "-sigmoidal-contrast", "4x48%", "-modulate", "100,122,100"]);
        }
        ColorProfile::Portrait => {
            cmd.args(["-auto-level", "-sigmoidal-contrast", "2.2x52%", "-modulate", "100,105,100"]);
        }
        ColorProfile::Cinematic => {
            cmd.args(["-auto-level", "-sigmoidal-contrast", "4.5x45%", "-modulate", "100,108,100"]);
        }
        ColorProfile::None => {}
    }

    cmd.args([
        "-colorspace", "sRGB",
        "-sampling-factor", "4:4:4",
        "-quality", &config.quality.to_string(),
        &output_path.display().to_string(),
    ]);

    let output = cmd.output().map_err(|e| DngError::ExternalDelegateFailed(format!("Failed to execute ImageMagick: {}", e)))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(DngError::ExternalDelegateFailed(err.trim().to_string()));
    }
    Ok(())
}

pub fn convert_single_dng(
    source_path: &Path,
    output_path: &Path,
    config: &ConversionConfig,
) -> ConversionResult {
    let start = Instant::now();
    let src_clean = match source_path.canonicalize() {
        Ok(p) => p,
        Err(_) => source_path.to_path_buf(),
    };

    let filename = src_clean.file_name().unwrap_or_default().to_string_lossy().to_string();
    let output_filename = output_path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let source_size = fs::metadata(&src_clean).map(|m| m.len()).unwrap_or(0);

    if let Some(parent) = output_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let tmp_path = output_path.with_file_name(format!(".tmp_{}_{}.jpg", std::process::id(), ts));

    let mut engine_used = String::new();
    let mut width = 0;
    let mut height = 0;

    let mut execute_conversion = || -> Result<(), DngError> {
        let bytes = fs::read(&src_clean).map_err(DngError::Io)?;
        match decode_dng_pure_rust(&bytes, config.auto_orient) {
            Ok((mut img, eng)) => {
                engine_used = eng;
                width = img.width();
                height = img.height();

                apply_adaptive_grading(&mut img, config.color_profile);

                let mut file = File::create(&tmp_path).map_err(DngError::Io)?;
                let mut encoder = JpegEncoder::new_with_quality(&mut file, config.quality);
                encoder.encode(img.as_raw(), width, height, image::ExtendedColorType::Rgb8)
                    .map_err(DngError::Image)?;
                file.flush().map_err(DngError::Io)?;
                Ok(())
            }
            Err(err) => {
                if config.allow_magick_fallback {
                    engine_used = format!("ImageMagick 7 Fallback ({})", err);
                    convert_with_magick_fallback(&src_clean, &tmp_path, config)?;
                    if let Ok(reader) = ImageReader::open(&tmp_path) {
                        if let Ok(dims) = reader.into_dimensions() {
                            width = dims.0;
                            height = dims.1;
                        }
                    }
                    Ok(())
                } else {
                    Err(err)
                }
            }
        }
    };

    let conversion_res = execute_conversion();

    match conversion_res {
        Ok(()) => {
            let _ = fs::rename(&tmp_path, output_path);
            let out_size = fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
            ConversionResult {
                source_path: src_clean.to_string_lossy().to_string(),
                output_path: output_path.to_string_lossy().to_string(),
                filename,
                output_filename,
                success: true,
                source_size_bytes: source_size,
                output_size_bytes: out_size,
                width,
                height,
                duration_sec: (start.elapsed().as_millis() as f64) / 1000.0,
                engine_used,
                error: None,
            }
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            ConversionResult {
                source_path: src_clean.to_string_lossy().to_string(),
                output_path: output_path.to_string_lossy().to_string(),
                filename,
                output_filename,
                success: false,
                source_size_bytes: source_size,
                output_size_bytes: 0,
                width: 0,
                height: 0,
                duration_sec: (start.elapsed().as_millis() as f64) / 1000.0,
                engine_used,
                error: Some(e.to_string()),
            }
        }
    }
}