# DNG to High-Quality JPEG Studio & Batch Converter

A high-precision 16-bit DNG (RAW Digital Negative) to high-quality JPEG batch processing engine and interactive web studio powered by **Python**, **ImageMagick 7**, and **LibRaw**.

Designed for photographers, action-cam users (DJI, Insta360), mobile shooters (Apple ProRAW), and digital archival workflows that require maximum sharpness, accurate color grading, and non-destructive batch conversion.

---

## 🌟 Key Features

- **16-Bit RAW Development**: Decodes full sensor dynamic range via ImageMagick 7 LibRaw or Python `rawpy`.
- **4:4:4 Chroma Subsampling**: Disables chroma subsampling for pin-sharp edges and zero color blur.
- **Near-Lossless JPEG Encoding**: Default 98% quality with optimized Huffman tables and sRGB color profile.
- **Full EXIF Metadata Preservation**: Copies camera metadata (shutter speed, aperture, ISO, lens, capture timestamp, and GPS) and applies auto-orientation.
- **Non-Destructive Atomic Writes**: Writes to `.tmp` files before renaming to destination paths—ensuring original `.dng` files are strictly read-only and partial conversions never corrupt files.
- **Real-Time Telemetry & SSE**: Live conversion progress bar, speed telemetry, before/after file size analytics, and interactive lightbox gallery.
- **Native OS Integration**: Native Windows Explorer folder and file picker dialogs, detected DNG folder quick-chips, and "Reveal in Explorer" buttons.

---

## 🚀 Execution Modes

### 1. Web Studio (Python + FastAPI)
The full interactive web UI with real-time SSE progress, gallery, and telemetry.
- **Quick Start**: Double-click `start_app.bat`
- **Manual Start**:
  ```bash
  pip install -r requirements.txt
  python app.py
  ```
- Opens in your browser at `http://127.0.0.1:8080`.

### 2. Standalone Windows Batch Script (`convert_dng_to_jpg.bat`)
A zero-dependency Windows script using installed ImageMagick 7:
- **Drag & Drop**: Drag one or more `.dng` files or folders directly onto `convert_dng_to_jpg.bat`.
- **In-Folder Execution**: Place the `.bat` file inside any folder containing `.dng` files and double-click it.
- **Command Line**:
  ```cmd
  convert_dng_to_jpg.bat "C:\Path\To\DNG_Folder"
  ```

### 3. Standalone Pure HTML/JS App (`standalone_dng_to_jpg.html`)
A 100% client-side web application that runs directly in Google Chrome or Microsoft Edge with **zero Python or backend server required**:
- Double-click `standalone_dng_to_jpg.html` to open via `file://`.
- Features direct disk read/write via the browser's File System Access API.

### 4. Modular Python CLI (`converter.py`)
Run batch conversions programmatically or headlessly:
```bash
# Convert a folder of DNGs using ImageMagick at 98% quality
python converter.py "C:\Photos\Raw_DNGs" -q 98 -e magick -s 4:4:4

# Convert using Python LibRaw (rawpy)
python converter.py "C:\Photos\Raw_DNGs" -q 98 -e rawpy
```

---

## 🛠️ Prerequisites & Installation

### Requirements
1. **Python 3.10+**: Available from [python.org](https://www.python.org/).
2. **ImageMagick 7 (Recommended)**:
   - Windows installer from [imagemagick.org](https://imagemagick.org/) (ensure **"Add application directory to your system path"** is checked).
   - Alternatively installed via WinGet: `winget install ImageMagick.ImageMagick`

### Python Setup
```bash
git clone https://github.com/jezehelfranca-svg/DNG_CONVERTER.git
cd DNG_CONVERTER
pip install -r requirements.txt
```

---

## 📁 Repository Structure

```
DNG_CONVERTER/
├── app.py                     # FastAPI backend server with SSE and worker pools
├── converter.py               # Core modular conversion engine (ImageMagick & rawpy)
├── convert_dng_to_jpg.bat     # Windows CMD drag-and-drop batch script
├── start_app.bat              # One-click launcher for the Web Studio
├── standalone_dng_to_jpg.html # 100% client-side HTML/JS converter (Zero Python)
├── static/
│   └── index.html             # Dark-mode responsive Web Studio frontend
├── requirements.txt           # Python package dependencies
├── .gitignore                 # Excludes raw DNGs, JPEGs, and upload caches
└── README.md                  # Project documentation
```

---

## 📄 License
MIT License. Free for personal, commercial, and studio use.