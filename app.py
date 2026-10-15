"""
app.py — FastAPI Backend for High-Quality DNG to JPEG Studio
Bound strictly to 127.0.0.1 with non-blocking worker pool and SSE telemetry.
"""

import asyncio
import io
import json
import os
import subprocess
import threading
import time
import uuid
import zipfile
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional

from fastapi import FastAPI, File, Form, HTTPException, Query, UploadFile
from fastapi.responses import FileResponse, HTMLResponse, StreamingResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel, Field

from converter import (
    HAS_RAWPY,
    MAGICK_EXE,
    ConversionConfig,
    ConversionResult,
    convert_single_dng,
)

BASE_DIR = Path(__file__).resolve().parent
STATIC_DIR = BASE_DIR / "static"
UPLOADS_DIR = BASE_DIR / "workspace_uploads"
STATIC_DIR.mkdir(parents=True, exist_ok=True)
UPLOADS_DIR.mkdir(parents=True, exist_ok=True)

app = FastAPI(
    title="DNG to High-Quality JPEG Studio",
    version="2.0.0",
    description="High-precision 16-bit DNG to JPEG batch processing web application",
)


# ---------------------------------------------------------------------------
# Data Models
# ---------------------------------------------------------------------------
class ScanRequest(BaseModel):
    path: str
    recursive: bool = False
    output_subfolder: str = "Converted_JPEG"


class StartJobRequest(BaseModel):
    files: List[str]
    engine: str = "magick"
    quality: int = Field(default=98, ge=1, le=100)
    subsampling: str = "4:4:4"
    colorspace: str = "sRGB"
    auto_orient: bool = True
    preserve_exif: bool = True
    output_subfolder: str = "Converted_JPEG"
    custom_output_dir: Optional[str] = None
    max_workers: int = Field(default=3, ge=1, le=8)


class ExplorerRequest(BaseModel):
    path: str
    select_file: bool = False


@dataclass
class BatchJob:
    job_id: str
    total_files: int
    config: ConversionConfig
    custom_output_dir: Optional[str] = None
    status: str = "running"  # "running", "completed", "cancelled"
    completed_count: int = 0
    success_count: int = 0
    fail_count: int = 0
    start_time: float = field(default_factory=time.time)
    end_time: Optional[float] = None
    current_files: List[str] = field(default_factory=list)
    results: List[dict] = field(default_factory=list)
    events: List[dict] = field(default_factory=list)
    cancel_requested: bool = False
    lock: threading.Lock = field(default_factory=threading.Lock)

    def push_event(self, event_type: str, payload: dict) -> None:
        with self.lock:
            self.events.append({
                "index": len(self.events),
                "type": event_type,
                "timestamp": time.time(),
                "data": payload,
            })

    def summary_dict(self) -> dict:
        elapsed = round((self.end_time or time.time()) - self.start_time, 2)
        return {
            "job_id": self.job_id,
            "status": self.status,
            "total_files": self.total_files,
            "completed_count": self.completed_count,
            "success_count": self.success_count,
            "fail_count": self.fail_count,
            "elapsed_sec": elapsed,
            "current_files": list(self.current_files),
            "results": list(self.results),
        }


JOBS: Dict[str, BatchJob] = {}
ALLOWED_PREVIEW_FILES: set[str] = set()


# ---------------------------------------------------------------------------
# Helper Functions
# ---------------------------------------------------------------------------
def discover_dng_folders() -> List[dict]:
    """Quickly scan common user picture folders for existing .dng files."""
    user_profile = Path(os.environ.get("USERPROFILE", str(Path.home())))
    candidate_roots = [
        user_profile / "OneDrive" / "Pictures",
        user_profile / "Pictures",
        user_profile / "Downloads",
        user_profile / "Desktop",
        BASE_DIR,
    ]
    folder_counts: Dict[str, int] = {}
    seen_roots = set()

    for root in candidate_roots:
        try:
            resolved_root = root.resolve()
        except Exception:
            continue
        if not resolved_root.is_dir() or str(resolved_root) in seen_roots:
            continue
        seen_roots.add(str(resolved_root))

        # Check top-level and up to depth 2
        try:
            for dng in resolved_root.glob("*.[dD][nN][gG]"):
                if dng.is_file():
                    key = str(dng.parent)
                    folder_counts[key] = folder_counts.get(key, 0) + 1
            for sub in resolved_root.iterdir():
                if sub.is_dir() and sub.name != "Converted_JPEG":
                    for dng in sub.glob("*.[dD][nN][gG]"):
                        if dng.is_file():
                            key = str(dng.parent)
                            folder_counts[key] = folder_counts.get(key, 0) + 1
                    for sub2 in sub.iterdir():
                        if sub2.is_dir() and sub2.name != "Converted_JPEG":
                            for dng in sub2.glob("*.[dD][nN][gG]"):
                                if dng.is_file():
                                    key = str(dng.parent)
                                    folder_counts[key] = folder_counts.get(key, 0) + 1
        except Exception:
            continue

    discovered = [
        {"path": k, "name": Path(k).name or k, "count": v}
        for k, v in folder_counts.items()
    ]
    discovered.sort(key=lambda x: (-x["count"], x["path"]))
    return discovered[:15]


def run_batch_job_worker(job: BatchJob, file_paths: List[Path], max_workers: int) -> None:
    """Background worker pool executing conversions off the async event loop."""
    job.push_event("job_started", {
        "job_id": job.job_id,
        "total_files": job.total_files,
        "engine": job.config.engine,
        "quality": job.config.quality,
        "subsampling": job.config.subsampling,
    })

    def process_one(src_path: Path) -> Optional[ConversionResult]:
        if job.cancel_requested:
            return None

        with job.lock:
            job.current_files.append(src_path.name)
        job.push_event("file_started", {
            "filename": src_path.name,
            "source_path": str(src_path),
        })

        if job.custom_output_dir:
            out_dir = Path(job.custom_output_dir).resolve()
        else:
            out_dir = src_path.parent / (job.config.output_subfolder or "Converted_JPEG")

        out_file = out_dir / f"{src_path.stem}.jpg"
        res = convert_single_dng(src_path, out_file, job.config)

        with job.lock:
            if src_path.name in job.current_files:
                job.current_files.remove(src_path.name)
            job.completed_count += 1
            if res.success:
                job.success_count += 1
                ALLOWED_PREVIEW_FILES.add(str(Path(res.output_path).resolve()))
            else:
                job.fail_count += 1
            res_dict = res.to_dict()
            job.results.append(res_dict)

        job.push_event("file_completed", {
            "result": res_dict,
            "completed_count": job.completed_count,
            "success_count": job.success_count,
            "fail_count": job.fail_count,
            "total_files": job.total_files,
        })
        return res

    with ThreadPoolExecutor(max_workers=max_workers) as pool:
        futures = [pool.submit(process_one, fp) for fp in file_paths]
        for fut in futures:
            try:
                fut.result()
            except Exception as exc:
                with job.lock:
                    job.completed_count += 1
                    job.fail_count += 1
                job.push_event("file_error", {"error": str(exc)})

    job.end_time = time.time()
    job.status = "cancelled" if job.cancel_requested else "completed"
    job.push_event("job_finished", job.summary_dict())


# ---------------------------------------------------------------------------
# API Endpoints
# ---------------------------------------------------------------------------
@app.get("/", response_class=HTMLResponse)
async def serve_index():
    index_file = STATIC_DIR / "index.html"
    if not index_file.is_file():
        return HTMLResponse("<h1>Frontend not found</h1>", status_code=404)
    return FileResponse(index_file)


@app.get("/api/system-info")
async def get_system_info():
    discovered = await asyncio.to_thread(discover_dng_folders)
    return {
        "has_magick": MAGICK_EXE is not None,
        "magick_path": MAGICK_EXE or "Not found",
        "has_rawpy": HAS_RAWPY,
        "cpu_count": os.cpu_count() or 4,
        "workspace_dir": str(BASE_DIR),
        "discovered_folders": discovered,
    }


@app.post("/api/pick-folder")
async def pick_folder():
    """Open native Windows folder dialog on a background thread."""
    def _open_dialog() -> str:
        import tkinter as tk
        from tkinter import filedialog
        root = tk.Tk()
        root.withdraw()
        root.attributes("-topmost", True)
        selected = filedialog.askdirectory(title="Select Folder Containing DNG Files")
        root.destroy()
        return selected or ""

    folder = await asyncio.to_thread(_open_dialog)
    return {"path": str(Path(folder).resolve()) if folder else ""}


@app.post("/api/pick-files")
async def pick_files():
    """Open native Windows file dialog for selecting multiple .dng files."""
    def _open_dialog() -> List[str]:
        import tkinter as tk
        from tkinter import filedialog
        root = tk.Tk()
        root.withdraw()
        root.attributes("-topmost", True)
        selected = filedialog.askopenfilenames(
            title="Select DNG Files to Convert",
            filetypes=[("Digital Negative RAW (*.dng)", "*.dng;*.DNG"), ("All Files", "*.*")],
        )
        root.destroy()
        return [str(Path(f).resolve()) for f in selected] if selected else []

    files = await asyncio.to_thread(_open_dialog)
    return {"files": files}


@app.post("/api/scan")
async def scan_directory(req: ScanRequest):
    raw_path = req.path.strip().strip('"').strip("'")
    if not raw_path:
        raise HTTPException(status_code=400, detail="Path cannot be empty.")

    target = Path(raw_path).resolve()
    if not target.exists():
        raise HTTPException(status_code=404, detail=f"Path does not exist: {target}")

    found_files: List[Path] = []
    if target.is_file():
        if target.suffix.lower() == ".dng":
            found_files.append(target)
    elif target.is_dir():
        pattern = "**/*.[dD][nN][gG]" if req.recursive else "*.[dD][nN][gG]"
        for p in sorted(target.glob(pattern)):
            if p.is_file() and req.output_subfolder not in p.parts:
                found_files.append(p)

    items = []
    total_bytes = 0
    for f in found_files:
        sz = f.stat().st_size
        total_bytes += sz
        out_jpg = f.parent / req.output_subfolder / f"{f.stem}.jpg"
        already_converted = out_jpg.is_file()
        if already_converted:
            ALLOWED_PREVIEW_FILES.add(str(out_jpg.resolve()))
        items.append({
            "name": f.name,
            "path": str(f),
            "folder": str(f.parent),
            "size_bytes": sz,
            "already_converted": already_converted,
            "expected_output": str(out_jpg),
        })

    return {
        "target_path": str(target),
        "count": len(items),
        "total_size_bytes": total_bytes,
        "files": items,
    }


@app.post("/api/jobs/start")
async def start_batch_job(req: StartJobRequest):
    valid_paths: List[Path] = []
    seen = set()
    for f_str in req.files:
        p = Path(f_str).resolve()
        if p.is_file() and p.suffix.lower() == ".dng" and str(p) not in seen:
            seen.add(str(p))
            valid_paths.append(p)

    if not valid_paths:
        raise HTTPException(status_code=400, detail="No valid .dng files provided for conversion.")

    cfg = ConversionConfig(
        engine=req.engine,
        quality=req.quality,
        subsampling=req.subsampling,
        colorspace=req.colorspace,
        auto_orient=req.auto_orient,
        preserve_exif=req.preserve_exif,
        output_subfolder=req.output_subfolder.strip() or "Converted_JPEG",
    )

    job_id = uuid.uuid4().hex[:12]
    job = BatchJob(
        job_id=job_id,
        total_files=len(valid_paths),
        config=cfg,
        custom_output_dir=req.custom_output_dir.strip() if req.custom_output_dir else None,
    )
    JOBS[job_id] = job

    worker_thread = threading.Thread(
        target=run_batch_job_worker,
        args=(job, valid_paths, req.max_workers),
        daemon=True,
    )
    worker_thread.start()

    return {"job_id": job_id, "total_files": len(valid_paths)}


@app.post("/api/jobs/upload")
async def upload_and_convert(
    files: List[UploadFile] = File(...),
    engine: str = Form("magick"),
    quality: int = Form(98),
    subsampling: str = Form("4:4:4"),
    colorspace: str = Form("sRGB"),
    auto_orient: bool = Form(True),
    preserve_exif: bool = Form(True),
    max_workers: int = Form(3),
):
    batch_id = time.strftime("%Y%m%d_%H%M%S") + "_" + uuid.uuid4().hex[:6]
    batch_dir = (UPLOADS_DIR / batch_id).resolve()
    batch_dir.mkdir(parents=True, exist_ok=True)

    saved_paths: List[Path] = []
    for uf in files:
        safe_name = Path(uf.filename or "image.dng").name
        if not safe_name.lower().endswith(".dng"):
            continue
        dest = (batch_dir / safe_name).resolve()
        content = await uf.read()
        with open(dest, "wb") as out_f:
            out_f.write(content)
        saved_paths.append(dest)

    if not saved_paths:
        raise HTTPException(status_code=400, detail="No valid .dng files uploaded.")

    cfg = ConversionConfig(
        engine=engine,
        quality=max(1, min(100, int(quality))),
        subsampling=subsampling,
        colorspace=colorspace,
        auto_orient=auto_orient,
        preserve_exif=preserve_exif,
        output_subfolder="Converted_JPEG",
    )

    job_id = uuid.uuid4().hex[:12]
    job = BatchJob(
        job_id=job_id,
        total_files=len(saved_paths),
        config=cfg,
    )
    JOBS[job_id] = job

    worker_thread = threading.Thread(
        target=run_batch_job_worker,
        args=(job, saved_paths, max(1, min(8, int(max_workers)))),
        daemon=True,
    )
    worker_thread.start()

    return {
        "job_id": job_id,
        "total_files": len(saved_paths),
        "upload_dir": str(batch_dir),
    }


@app.get("/api/jobs/{job_id}")
async def get_job_status(job_id: str):
    job = JOBS.get(job_id)
    if not job:
        raise HTTPException(status_code=404, detail="Job not found.")
    return job.summary_dict()


@app.post("/api/jobs/{job_id}/cancel")
async def cancel_job(job_id: str):
    job = JOBS.get(job_id)
    if not job:
        raise HTTPException(status_code=404, detail="Job not found.")
    job.cancel_requested = True
    return {"status": "cancelling"}


@app.get("/api/jobs/{job_id}/stream")
async def stream_job_events(job_id: str):
    job = JOBS.get(job_id)
    if not job:
        raise HTTPException(status_code=404, detail="Job not found.")

    async def event_generator():
        cursor = 0
        while True:
            with job.lock:
                pending = job.events[cursor:]
                cursor = len(job.events)
                current_status = job.status

            for ev in pending:
                yield f"data: {json.dumps(ev)}\n\n"

            if current_status in ("completed", "cancelled") and not pending:
                break

            await asyncio.sleep(0.2)

    return StreamingResponse(
        event_generator(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
        },
    )


@app.get("/api/preview")
async def preview_converted_jpeg(path: str = Query(...), download: bool = Query(False)):
    resolved = Path(path).resolve()
    if not resolved.is_file() or resolved.suffix.lower() not in (".jpg", ".jpeg"):
        raise HTTPException(status_code=404, detail="JPEG file not found.")
    if str(resolved) not in ALLOWED_PREVIEW_FILES:
        raise HTTPException(status_code=403, detail="Access denied to non-registered file.")

    headers = {}
    if download:
        headers["Content-Disposition"] = f'attachment; filename="{resolved.name}"'
    return FileResponse(resolved, media_type="image/jpeg", headers=headers)


@app.get("/api/download-zip/{job_id}")
async def download_job_zip(job_id: str):
    job = JOBS.get(job_id)
    if not job:
        raise HTTPException(status_code=404, detail="Job not found.")

    successful_files = [
        Path(r["output_path"]).resolve()
        for r in job.results
        if r.get("success") and Path(r["output_path"]).is_file()
    ]
    if not successful_files:
        raise HTTPException(status_code=400, detail="No converted JPEG files available to zip.")

    zip_buffer = io.BytesIO()
    with zipfile.ZipFile(zip_buffer, "w", zipfile.ZIP_STORED) as zf:
        for fpath in successful_files:
            zf.write(fpath, arcname=fpath.name)
    zip_buffer.seek(0)

    filename = f"Converted_JPEG_{job_id}.zip"
    return StreamingResponse(
        zip_buffer,
        media_type="application/zip",
        headers={"Content-Disposition": f'attachment; filename="{filename}"'},
    )


@app.post("/api/open-explorer")
async def open_in_explorer(req: ExplorerRequest):
    resolved = Path(req.path).resolve()
    if not resolved.exists():
        raise HTTPException(status_code=404, detail=f"Path not found: {resolved}")

    if os.name == "nt":
        if req.select_file and resolved.is_file():
            subprocess.Popen(["explorer.exe", f"/select,{resolved}"], shell=False)
        else:
            target_dir = resolved if resolved.is_dir() else resolved.parent
            subprocess.Popen(["explorer.exe", str(target_dir)], shell=False)
    return {"opened": str(resolved)}


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="127.0.0.1", port=8080, log_level="info")