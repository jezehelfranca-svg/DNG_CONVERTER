use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use rayon::prelude::*;
use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::color::ColorProfile;
use crate::engine::{convert_single_dng, ConversionConfig, ConversionResult};

static EMBEDDED_HTML: &str = include_str!("../../static/index.html");

#[derive(serde::Deserialize)]
struct ScanRequest {
    path: String,
    #[serde(default)]
    recursive: bool,
    #[serde(default = "default_subfolder")]
    output_subfolder: String,
}

fn default_subfolder() -> String {
    "Converted_JPEG".to_string()
}

#[derive(serde::Deserialize)]
struct StartJobRequest {
    files: Vec<String>,
    #[serde(default = "default_quality")]
    quality: u8,
    #[serde(default = "default_profile")]
    color_profile: String,
    #[serde(default = "default_true")]
    auto_orient: bool,
    #[serde(default = "default_subfolder")]
    output_subfolder: String,
}

fn default_quality() -> u8 { 98 }
fn default_profile() -> String { "balanced".to_string() }
fn default_true() -> bool { true }

#[derive(serde::Deserialize)]
struct ExplorerRequest {
    path: String,
    #[serde(default)]
    select_file: bool,
}

pub fn run_server(port: u16) {
    let addr = format!("127.0.0.1:{}", port);
    let server = match Server::http(&addr) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("[ERROR] Could not bind HTTP server to {}: {}", addr, e);
            return;
        }
    };

    println!("============================================================================");
    println!("  DNG to High-Quality JPEG Studio (Rust Engine)");
    println!("  URL: http://{}", addr);
    println!("============================================================================");

    // Open browser automatically
    let _ = Command::new("cmd")
        .args(["/C", "start", "", &format!("http://{}", addr)])
        .spawn();

    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().clone();

        if method == Method::Get && (url == "/" || url == "/index.html") {
            let resp = Response::from_string(EMBEDDED_HTML)
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap());
            let _ = request.respond(resp);
            continue;
        }

        if method == Method::Get && url.starts_with("/api/system-info") {
            let has_magick = Command::new("magick").arg("-version").output().is_ok();
            let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
            let json = serde_json::json!({
                "has_magick": has_magick,
                "has_rawpy": false,
                "cpu_count": cpus,
                "engine_type": "Rust SIMD + Oklab Color Grading",
            });
            let resp = Response::from_string(json.to_string())
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
            let _ = request.respond(resp);
            continue;
        }

        if method == Method::Post && url.starts_with("/api/scan") {
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_ok() {
                if let Ok(req) = serde_json::from_str::<ScanRequest>(&body) {
                    let target = Path::new(&req.path);
                    let mut items = Vec::new();

                    if target.is_dir() {
                        let walker = walkdir::WalkDir::new(target)
                            .max_depth(if req.recursive { 8 } else { 1 });

                        for entry in walker.into_iter().filter_map(|e| e.ok()) {
                            let p = entry.path();
                            if p.is_file() {
                                if let Some(ext) = p.extension() {
                                    if ext.to_string_lossy().eq_ignore_ascii_case("dng") {
                                        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                                        let size_bytes = p.metadata().map(|m| m.len()).unwrap_or(0);
                                        let parent = p.parent().unwrap_or(target);
                                        let out_jpg = parent.join(&req.output_subfolder).join(format!("{}.jpg", p.file_stem().unwrap_or_default().to_string_lossy()));
                                        let already_converted = out_jpg.is_file();

                                        items.push(serde_json::json!({
                                            "name": name,
                                            "path": p.to_string_lossy(),
                                            "folder": parent.to_string_lossy(),
                                            "size_bytes": size_bytes,
                                            "already_converted": already_converted,
                                        }));
                                    }
                                }
                            }
                        }
                    }

                    let resp_json = serde_json::json!({
                        "count": items.len(),
                        "files": items,
                    });
                    let resp = Response::from_string(resp_json.to_string())
                        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }
            }
        }

        if method == Method::Post && url.starts_with("/api/jobs/start") {
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_ok() {
                if let Ok(req) = serde_json::from_str::<StartJobRequest>(&body) {
                    let cfg = ConversionConfig {
                        quality: req.quality.clamp(1, 100),
                        color_profile: ColorProfile::from_str_loose(&req.color_profile),
                        auto_orient: req.auto_orient,
                        output_subfolder: req.output_subfolder,
                        allow_magick_fallback: true,
                    };

                    // Run parallel conversion with Rayon
                    let results: Vec<ConversionResult> = req.files.par_iter().map(|file_str| {
                        let src_path = Path::new(file_str);
                        let stem = src_path.file_stem().unwrap_or_default().to_string_lossy();
                        let parent = src_path.parent().unwrap_or(Path::new("."));
                        let out_dir = parent.join(&cfg.output_subfolder);
                        let out_path = out_dir.join(format!("{}.jpg", stem));

                        convert_single_dng(src_path, &out_path, &cfg)
                    }).collect();

                    let resp_json = serde_json::json!({
                        "job_id": "rust_job",
                        "total_files": results.len(),
                        "results": results,
                    });
                    let resp = Response::from_string(resp_json.to_string())
                        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }
            }
        }

        if method == Method::Get && url.starts_with("/api/preview") {
            if let Some(query) = url.split('?').nth(1) {
                let params: Vec<(&str, &str)> = query.split('&').filter_map(|p| {
                    let mut parts = p.split('=');
                    Some((parts.next()?, parts.next()?))
                }).collect();

                if let Some((_, raw_path)) = params.iter().find(|(k, _)| *k == "path") {
                    let decoded_path = raw_path.replace("%2F", "/").replace("%5C", "\\").replace("%20", " ");
                    let path = Path::new(&decoded_path);
                    if path.is_file() {
                        if let Ok(mut f) = File::open(path) {
                            let mut buf = Vec::new();
                            if f.read_to_end(&mut buf).is_ok() {
                                let resp = Response::from_data(buf)
                                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"image/jpeg"[..]).unwrap());
                                let _ = request.respond(resp);
                                continue;
                            }
                        }
                    }
                }
            }
        }

        if method == Method::Post && url.starts_with("/api/open-explorer") {
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_ok() {
                if let Ok(req) = serde_json::from_str::<ExplorerRequest>(&body) {
                    let p = Path::new(&req.path);
                    if p.exists() {
                        if req.select_file && p.is_file() {
                            let _ = Command::new("explorer").args(["/select,", &p.to_string_lossy()]).spawn();
                        } else {
                            let target = if p.is_dir() { p } else { p.parent().unwrap_or(p) };
                            let _ = Command::new("explorer").arg(target).spawn();
                        }
                    }
                    let resp = Response::from_string(r#"{"status":"ok"}"#)
                        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }
            }
        }

        // 404 Fallback
        let _ = request.respond(Response::from_string("Not Found").with_status_code(StatusCode(404)));
    }
}