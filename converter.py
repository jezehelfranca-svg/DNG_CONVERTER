"""
converter.py — Modular High-Quality DNG to JPEG Conversion Engine
Adheres to NEXUS SOB Directives:
- Strictly read-only access on source .dng files
- Atomic write-then-rename (.tmp.jpg -> .jpg)
- Parameterized subprocess execution (shell=False)
- Explicit garbage collection per file
- Full telemetry (dimensions, sizes, duration, engine used)
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
    from PIL import Image, ImageOps
    HAS_RAWPY = True
except ImportError:
    HAS_RAWPY = False


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
    engine: str = "magick"          # "magick" or "rawpy"
    quality: int = 98               # 1 - 100
    subsampling: str = "4:4:4"      # "4:4:4", "4:2:2", "4:2:0"
    colorspace: str = "sRGB"        # "sRGB" or "AdobeRGB"
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

    # Atomic temporary output file in the same target directory
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
            used_engine = "Python LibRaw (rawpy + Pillow)"
        else:
            _convert_with_magick(src_resolved, tmp_output, config)
            used_engine = "ImageMagick 7 (16-bit LibRaw)"

        if not tmp_output.is_file() or tmp_output.stat().st_size == 0:
            raise RuntimeError("Conversion produced an empty or missing output file.")

        # Atomic move into final destination
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
    """Run ImageMagick safely via parameterized list (shell=False)."""
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
    """Demosaic DNG using LibRaw (rawpy) and encode high-quality JPEG via Pillow."""
    # Open strictly read-only binary stream
    with open(src_path, "rb") as f:
        with rawpy.imread(f) as raw:
            output_cs = (
                rawpy.ColorSpace.Adobe if config.colorspace == "AdobeRGB" else rawpy.ColorSpace.sRGB
            )
            rgb = raw.postprocess(
                use_camera_wb=True,
                half_size=False,
                no_auto_bright=False,
                output_bps=8,
                output_color=output_cs,
                demosaic_algorithm=rawpy.DemosaicAlgorithm.AHD
            )

    img = Image.fromarray(rgb)
    del rgb

    # Extract EXIF from original DNG/TIFF header if preserve_exif is enabled
    exif_bytes = None
    if config.preserve_exif:
        try:
            with Image.open(src_path) as src_img:
                if config.auto_orient:
                    img = ImageOps.exif_transpose(img)
                raw_exif = src_img.getexif()
                if raw_exif:
                    # Reset Orientation tag to 1 (Normal) since pixels are already oriented
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
    parser.add_argument("-s", "--subsampling", choices=["4:4:4", "4:2:2", "4:2:0"], default="4:4:4")
    parser.add_argument("-o", "--output-subfolder", default="Converted_JPEG")
    args = parser.parse_args()

    cfg = ConversionConfig(
        engine=args.engine,
        quality=args.quality,
        subsampling=args.subsampling,
        output_subfolder=args.output_subfolder
    )

    files_to_process: list[Path] = []
    for item in args.inputs:
        p = Path(item).resolve()
        if p.is_dir():
            files_to_process.extend(sorted(p.glob("*.[dD][nN][gG]")))
        elif p.is_file() and p.suffix.lower() == ".dng":
            files_to_process.append(p)

    print(f"Found {len(files_to_process)} DNG file(s) to convert.")
    for idx, fpath in enumerate(files_to_process, 1):
        out_dir = fpath.parent / cfg.output_subfolder
        out_file = out_dir / f"{fpath.stem}.jpg"
        print(f"[{idx}/{len(files_to_process)}] Converting {fpath.name} -> {out_file} ...", end=" ", flush=True)
        res = convert_single_dng(fpath, out_file, cfg)
        if res.success:
            print(f"OK ({res.width}x{res.height}, {res.output_size_bytes / (1024*1024):.2f} MB, {res.duration_sec}s)")
        else:
            print(f"FAILED: {res.error}")