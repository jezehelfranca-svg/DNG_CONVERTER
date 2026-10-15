"""
converter.py — Modular High-Quality DNG to JPEG Conversion Engine
Features Adaptive Color Optimization:
- As-Shot Camera Calibration & White Balance (Camera Matrix)
- LAB-Space CLAHE Adaptive Tone Mapping (Dynamic Shadow/Highlight Recovery)
- Smart Vibrance (Saturation boost protecting skin tones)
- ImageMagick 7 Filmic S-Curves & Parameterized Execution
- Atomic write-then-rename (.tmp.jpg -> .jpg)
"""

import gc
import os
import shutil
import subprocess
import time
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Optional

try:
    import rawpy
    from PIL import Image, ImageOps, ImageEnhance
    HAS_RAWPY = True
except ImportError:
    HAS_RAWPY = False

try:
    import cv2
    import numpy as np
    HAS_CV2 = True
except ImportError:
    HAS_CV2 = False


def find_magick_executable() -> Optional[str]:
    """Locate ImageMagick executable safely without shell invocation."""
    which_magick = shutil.which("magick")
    if which_magick:
        return which_magick
    default_paths = [
        r"C:\Program Files\ImageMagick-7.1.2-Q16-HDRI\magick.exe",
        r"C:\Program Files\ImageMagick-7.1.1-Q16-HDRI\magick.exe",
    ]
    for p in default_paths:
        if os.path.isfile(p):
            return p
    return None


MAGICK_EXE = find_magick_executable()


@dataclass
class ConversionConfig:
    engine: str = "magick"             # "magick" or "rawpy"
    quality: int = 98                  # 1 - 100
    subsampling: str = "4:4:4"         # "4:4:4", "4:2:2", "4:2:0"
    colorspace: str = "sRGB"           # "sRGB" or "AdobeRGB"
    color_profile: str = "balanced"    # "balanced", "vibrant", "portrait", "cinematic", "none"
    auto_orient: bool = True
    preserve_exif: bool = True
    output_subfolder: str = "Converted_JPEG"
    overwrite: bool = True


@dataclass
class ConversionResult:
    source_path: str
    output_path: str
    filename: str
    output_filename: str
    success: bool
    source_size_bytes: int = 0
    output_size_bytes: int = 0
    width: int = 0
    height: int = 0
    duration_sec: float = 0.0
    engine_used: str = ""
    error: Optional[str] = None

    def to_dict(self) -> dict:
        return asdict(self)


def get_jpeg_dimensions(jpg_path: Path) -> tuple[int, int]:
    """Read width and height of generated JPEG cleanly."""
    try:
        from PIL import Image
        with Image.open(jpg_path) as img:
            return img.width, img.height
    except Exception:
        return 0, 0


def apply_lab_color_grading(rgb_array: "np.ndarray", profile: str) -> "np.ndarray":
    """
    Applies adaptive color optimization in LAB & HSV color spaces.
    Modulates Lightness without shifting hues; boosts vibrance while protecting skin tones.
    """
    if not HAS_CV2 or profile == "none":
        return rgb_array

    # 1. LAB CLAHE for adaptive local tone mapping
    lab = cv2.cvtColor(rgb_array, cv2.COLOR_RGB2LAB)
    l_channel, a_channel, b_channel = cv2.split(lab)

    clip_limits = {
        "balanced": 2.0,
        "vibrant": 2.8,
        "portrait": 1.3,
        "cinematic": 2.2,
    }
    clip = clip_limits.get(profile, 2.0)
    clahe = cv2.createCLAHE(clipLimit=clip, tileGridSize=(8, 8))
    l_graded = clahe.apply(l_channel)

    # Subtle cinematic warmth if requested
    if profile == "cinematic":
        b_channel = np.clip(b_channel.astype(np.int16) + 2, 0, 255).astype(np.uint8)

    merged_lab = cv2.merge((l_graded, a_channel, b_channel))
    rgb_graded = cv2.cvtColor(merged_lab, cv2.COLOR_LAB2RGB)

    # 2. Smart Vibrance: boost muted colors more than already-saturated colors
    hsv = cv2.cvtColor(rgb_graded, cv2.COLOR_RGB2HSV).astype(np.float32)
    sat = hsv[:, :, 1]

    vib_strengths = {
        "balanced": 16.0,
        "vibrant": 26.0,
        "portrait": 6.0,
        "cinematic": 10.0,
    }
    strength = vib_strengths.get(profile, 16.0)

    # Calculate inverse saturation weight (muted colors get boosted, saturated skin stays intact)
    weight = 1.0 - (sat / 255.0)
    sat += weight * strength
    hsv[:, :, 1] = np.clip(sat, 0, 255)

    return cv2.cvtColor(hsv.astype(np.uint8), cv2.COLOR_HSV2RGB)


def convert_single_dng(
    source_file: Path,
    output_file: Path,
    config: ConversionConfig
) -> ConversionResult:
    """
    Convert a single .dng file to a high-quality .jpg using atomic write-then-rename.
    Never modifies the source .dng file.
    """
    start_time = time.perf_counter()
    src_resolved = source_file.resolve()
    out_resolved = output_file.resolve()

    if not src_resolved.is_file():
        return ConversionResult(
            source_path=str(src_resolved),
            output_path=str(out_resolved),
            filename=src_resolved.name,
            output_filename=out_resolved.name,
            success=False,
            error="Source DNG file does not exist."
        )

    source_size = src_resolved.stat().st_size
    out_resolved.parent.mkdir(parents=True, exist_ok=True)

    tmp_output = out_resolved.with_name(f".tmp_{os.getpid()}_{ int(time.time() * 1000) }_{out_resolved.name}")

    engine = config.engine.lower()
    if engine == "magick" and not MAGICK_EXE:
        if HAS_RAWPY:
            engine = "rawpy"
        else:
            return ConversionResult(
                source_path=str(src_resolved),
                output_path=str(out_resolved),
                filename=src_resolved.name,
                output_filename=out_resolved.name,
                success=False,
                source_size_bytes=source_size,
                error="Neither ImageMagick (magick.exe) nor rawpy is available."
            )

    try:
        if engine == "rawpy" and HAS_RAWPY:
            _convert_with_rawpy(src_resolved, tmp_output, config)
            used_engine = f"Python LibRaw ({config.color_profile})"
        else:
            _convert_with_magick(src_resolved, tmp_output, config)
            used_engine = f"ImageMagick 7 ({config.color_profile})"

        if not tmp_output.is_file() or tmp_output.stat().st_size == 0:
            raise RuntimeError("Conversion produced an empty or missing output file.")

        os.replace(str(tmp_output), str(out_resolved))

        out_size = out_resolved.stat().st_size
        w, h = get_jpeg_dimensions(out_resolved)
        elapsed = round(time.perf_counter() - start_time, 2)

        return ConversionResult(
            source_path=str(src_resolved),
            output_path=str(out_resolved),
            filename=src_resolved.name,
            output_filename=out_resolved.name,
            success=True,
            source_size_bytes=source_size,
            output_size_bytes=out_size,
            width=w,
            height=h,
            duration_sec=elapsed,
            engine_used=used_engine
        )

    except Exception as exc:
        if tmp_output.exists():
            try:
                tmp_output.unlink()
            except OSError:
                pass
        elapsed = round(time.perf_counter() - start_time, 2)
        return ConversionResult(
            source_path=str(src_resolved),
            output_path=str(out_resolved),
            filename=src_resolved.name,
            output_filename=out_resolved.name,
            success=False,
            source_size_bytes=source_size,
            duration_sec=elapsed,
            engine_used=engine,
            error=str(exc)
        )
    finally:
        gc.collect()


def _convert_with_magick(src_path: Path, tmp_output: Path, config: ConversionConfig) -> None:
    """Run ImageMagick with adaptive filmic curves and tone optimization."""
    if not MAGICK_EXE:
        raise RuntimeError("ImageMagick executable not found.")

    cmd = [
        MAGICK_EXE,
        f"{src_path}[0]",
    ]
    if config.auto_orient:
        cmd.append("-auto-orient")
    if not config.preserve_exif:
        cmd.append("-strip")

    # Adaptive tone curve & contrast profile
    prof = (config.color_profile or "balanced").lower()
    if prof == "balanced":
        cmd.extend(["-auto-level", "-sigmoidal-contrast", "3x50%", "-modulate", "100,112,100"])
    elif prof == "vibrant":
        cmd.extend(["-auto-level", "-sigmoidal-contrast", "4x48%", "-modulate", "100,122,100"])
    elif prof == "portrait":
        cmd.extend(["-auto-level", "-sigmoidal-contrast", "2.2x52%", "-modulate", "100,105,100"])
    elif prof == "cinematic":
        cmd.extend(["-auto-level", "-sigmoidal-contrast", "4.5x45%", "-modulate", "100,108,100"])

    cmd.extend([
        "-colorspace", config.colorspace if config.colorspace in ("sRGB", "AdobeRGB") else "sRGB",
        "-sampling-factor", config.subsampling if config.subsampling in ("4:4:4", "4:2:2", "4:2:0") else "4:4:4",
        "-quality", str(max(1, min(100, int(config.quality)))),
        str(tmp_output)
    ])

    creationflags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
    proc = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        shell=False,
        creationflags=creationflags,
        timeout=180
    )
    if proc.returncode != 0:
        err_msg = (proc.stderr or proc.stdout or f"ImageMagick exited with code {proc.returncode}").strip()
        raise RuntimeError(err_msg)


def _convert_with_rawpy(src_path: Path, tmp_output: Path, config: ConversionConfig) -> None:
    """Demosaic DNG using LibRaw (rawpy) with camera WB, color matrix, and LAB CLAHE."""
    with open(src_path, "rb") as f:
        with rawpy.imread(f) as raw:
            output_cs = (
                rawpy.ColorSpace.Adobe if config.colorspace == "AdobeRGB" else rawpy.ColorSpace.sRGB
            )
            # Postprocess with Camera White Balance, Camera Matrix & Highlight Blend
            rgb = raw.postprocess(
                use_camera_wb=True,
                use_camera_matrix=True,
                highlight_mode=rawpy.HighlightMode.Blend,
                half_size=False,
                no_auto_bright=False,
                output_bps=8,
                output_color=output_cs,
                demosaic_algorithm=rawpy.DemosaicAlgorithm.AHD
            )

    # Apply adaptive LAB-space color optimization
    prof = (config.color_profile or "balanced").lower()
    if HAS_CV2 and prof != "none":
        rgb = apply_lab_color_grading(rgb, prof)

    img = Image.fromarray(rgb)
    del rgb

    # Pillow fallback if cv2 was unavailable
    if not HAS_CV2 and prof != "none":
        enhancers = {
            "balanced": (1.12, 1.12),
            "vibrant": (1.20, 1.25),
            "portrait": (1.05, 1.05),
            "cinematic": (1.18, 1.08),
        }
        con_mult, col_mult = enhancers.get(prof, (1.12, 1.12))
        img = ImageEnhance.Contrast(img).enhance(con_mult)
        img = ImageEnhance.Color(img).enhance(col_mult)

    # Extract and preserve original camera EXIF
    exif_bytes = None
    if config.preserve_exif:
        try:
            with Image.open(src_path) as src_img:
                if config.auto_orient:
                    img = ImageOps.exif_transpose(img)
                raw_exif = src_img.getexif()
                if raw_exif:
                    if config.auto_orient and 0x0112 in raw_exif:
                        raw_exif[0x0112] = 1
                    exif_bytes = raw_exif.tobytes()
        except Exception:
            exif_bytes = None

    subsampling_map = {
        "4:4:4": 0,
        "4:2:2": 1,
        "4:2:0": 2,
    }
    subsampling_val = subsampling_map.get(config.subsampling, 0)
    quality_val = max(1, min(100, int(config.quality)))

    save_kwargs = {
        "format": "JPEG",
        "quality": quality_val,
        "subsampling": subsampling_val,
        "optimize": True,
    }
    if exif_bytes:
        save_kwargs["exif"] = exif_bytes

    img.save(tmp_output, **save_kwargs)
    img.close()


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="High-Quality DNG to JPEG CLI Converter")
    parser.add_argument("inputs", nargs="+", help="Input .dng files or directories")
    parser.add_argument("-q", "--quality", type=int, default=98, help="JPEG quality (1-100, default: 98)")
    parser.add_argument("-e", "--engine", choices=["magick", "rawpy"], default="magick", help="Conversion engine")
    parser.add_argument("-c", "--color-profile", choices=["balanced", "vibrant", "portrait", "cinematic", "none"], default="balanced", help="Color optimization profile")
    parser.add_argument("-s", "--subsampling", choices=["4:4:4", "4:2:2", "4:2:0"], default="4:4:4")
    parser.add_argument("-o", "--output-subfolder", default="Converted_JPEG")
    args = parser.parse_args()

    cfg = ConversionConfig(
        engine=args.engine,
        quality=args.quality,
        subsampling=args.subsampling,
        color_profile=args.color_profile,
        output_subfolder=args.output_subfolder
    )

    files_to_process: list[Path] = []
    for item in args.inputs:
        p = Path(item).resolve()
        if p.is_dir():
            files_to_process.extend(sorted(p.glob("*.[dD][nN][gG]")))
        elif p.is_file() and p.suffix.lower() == ".dng":
            files_to_process.append(p)

    print(f"Found {len(files_to_process)} DNG file(s) to convert with [{cfg.color_profile}] color grading.")
    for idx, fpath in enumerate(files_to_process, 1):
        out_dir = fpath.parent / cfg.output_subfolder
        out_file = out_dir / f"{fpath.stem}.jpg"
        print(f"[{idx}/{len(files_to_process)}] Converting {fpath.name} -> {out_file.name} ...", end=" ", flush=True)
        res = convert_single_dng(fpath, out_file, cfg)
        if res.success:
            print(f"OK ({res.width}x{res.height}, {res.output_size_bytes / (1024*1024):.2f} MB, {res.duration_sec}s)")
        else:
            print(f"FAILED: {res.error}")