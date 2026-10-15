# DNG to High-Quality JPEG Studio & Batch Converter

A high-precision 16-bit DNG (RAW Digital Negative) to high-quality JPEG batch processing engine and interactive web studio available in **pure Rust**, **Python + ImageMagick**, and **standalone client-side HTML/JS**.

Designed for photographers, action-cam users (DJI, Insta360), mobile shooters (Apple ProRAW), and digital archival workflows that require maximum sharpness, accurate color grading, and non-destructive batch conversion.

---

## 🦀 Pure Rust Standalone Engine (`rust/`)

A 100% self-contained, blazing-fast native binary compiled into a single **1.8 MB `dng_converter.exe`** with **zero runtime dependencies** (no Python, no Node, no ImageMagick required):

- **Pure-Rust Pipeline**: Pure-Rust TIFF/DNG parser with embedded camera preview stream extraction and 16-bit TIFF raster decoder.
- **Oklab Perceptually Uniform Color Grading**: Pure-Rust color science in Oklab space for filmic Sigmoidal S-curves and Smart Vibrance with zero hue shifting.
- **Rayon Parallelism**: Multi-threaded SIMD execution utilizing all available CPU cores with bounded memory safeguards.
- **Embedded Web Studio**: The complete HTML/CSS/JS Web Studio is compiled directly inside the binary memory—double-click `dng_converter.exe` or run with `--serve` to start the local web interface!

### Build & Run the Rust Version
```bash
# 1-Click Build (creates rust/target/release/dng_converter.exe):
build_rust.bat

# 1-Click Launch Web Studio:
run_rust_studio.bat

# Or via Cargo CLI:
cd rust
cargo build --release
.\target\release\dng_converter.exe --serve
```

### Rust CLI Batch Usage
```bash
# Convert folder using Auto-Balanced Oklab color grading
.\dng_converter.exe "C:\Photos\Raw_DNGs" -q 98 -c balanced

# Convert action-cam DNGs using Vibrant profile with 4 threads
.\dng_converter.exe "C:\Photos\Insta360" -c vibrant -q 98 -t 4
```

---

## 🎨 Adaptive Color Optimization (Zero-LUT Dynamic Tone)

Instead of rigid `.cube` LUTs that break across different lighting conditions, the engine uses **Adaptive Dynamic Grading**:

| Profile | Best For | Technical Recipe |
| :--- | :--- | :--- |
| **🌟 Auto-Balanced** *(Default)* | Everyday scenes, travel, street | Camera WB + Oklab Sigmoidal S-curve ($k=2.4$) + Smart Vibrance (or 2.0x LAB CLAHE) |
| **🏞️ Vibrant / Outdoor** | Insta360, DJI, GoPro action, landscapes | Richer sky blues, foliage pop, punchy contrast ($k=3.2$, +26% Vibrance) |
| **👤 Natural / Portrait** | People, skin tones, weddings | Gentle shadow roll-off, soft skin protection ($k=1.6$, +6% Vibrance) |
| **🎬 Cinematic Filmic** | Moody, atmospheric, golden hour | Deep blacks, compressed highlights, subtle warm tint |
| **⚪ Off / Flat RAW** | Manual post-grading in Lightroom/Photoshop | Pure linear uncurved sensor demosaicing |

---

## 🚀 Execution Modes

### 1. Standalone Rust Executable (`dng_converter.exe`)
- **Web Studio Mode**: Run with `--serve` (or double-click) to open `http://127.0.0.1:8080`.
- **CLI Batch Mode**: Run `dng_converter.exe "C:\Path\To\DNGs" -c balanced -q 98`.

### 2. Python Web Studio (`app.py` + `start_app.bat`)
- The FastAPI backend web application with SSE real-time streaming and native Windows dialogs.
  ```bash
  pip install -r requirements.txt
  python app.py
  ```

### 3. Standalone Windows Batch Script (`convert_dng_to_jpg.bat`)
- Zero-install Windows script using ImageMagick 7:
  - Drag & drop `.dng` files or folders directly onto `convert_dng_to_jpg.bat`.

### 4. Standalone Pure HTML/JS App (`standalone_dng_to_jpg.html`)
- 100% client-side web application running directly in Google Chrome or Microsoft Edge with **zero runtime required**:
  - Double-click `standalone_dng_to_jpg.html` to open via `file://`.
  - Features direct disk read/write via the browser's File System Access API.

---

## 📁 Repository Structure

```
DNG_CONVERTER/
├── rust/                      # Pure Rust standalone application
│   ├── Cargo.toml             # Rust package configuration
│   └── src/
│       ├── main.rs            # CLI & Web Studio launcher
│       ├── engine.rs          # Pure-Rust TIFF/DNG decoder & atomic writer
│       ├── color.rs           # Oklab perceptually uniform color grading
│       ├── server.rs          # Embedded HTTP server with embedded frontend
│       └── error.rs           # Typed error definitions
├── build_rust.bat             # 1-Click release build script for Rust binary
├── run_rust_studio.bat        # 1-Click launcher for Rust Web Studio
├── app.py                     # FastAPI Python backend server
├── converter.py               # Modular Python engine (ImageMagick & rawpy)
├── convert_dng_to_jpg.bat     # Windows CMD drag-and-drop batch script
├── start_app.bat              # One-click launcher for Python Web Studio
├── standalone_dng_to_jpg.html # 100% client-side HTML/JS converter (Zero Python)
├── static/
│   └── index.html             # Dark-mode responsive Web Studio frontend
├── requirements.txt           # Python package dependencies
├── .gitignore                 # Excludes raw DNGs, target/ builds, and caches
└── README.md                  # Project documentation
```

---

## 📄 License
MIT License. Free for personal, commercial, and studio use.