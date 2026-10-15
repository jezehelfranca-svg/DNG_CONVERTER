use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rayon::prelude::*;
use tiny_http::{Header, Method, Response, Server, StatusCode};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

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

struct UploadedFile {
    filename: String,
    data: Vec<u8>,
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn extract_attr(line: &str, attr: &str) -> Option<String> {
    let key = format!("{}=\"", attr);
    if let Some(idx) = line.find(&key) {
        let rest = &line[idx + key.len()..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

fn parse_multipart(body: &[u8], boundary: &[u8]) -> (Vec<UploadedFile>, HashMap<String, String>) {
    let mut files = Vec::new();
    let mut fields = HashMap::new();

    let full_boundary = [b"--", boundary].concat();

    let mut start_idx = 0;
    while let Some(pos) = find_subsequence(&body[start_idx..], &full_boundary) {
        let part_start = start_idx + pos + full_boundary.len();
        if part_start + 2 > body.len() { break; }
        let actual_start = if &body[part_start..part_start + 2] == b"\r\n" { part_start + 2 } else { part_start };

        let next_bound = match find_subsequence(&body[actual_start..], &full_boundary) {
            Some(p) => actual_start + p,
            None => body.len(),
        };

        let part_bytes = &body[actual_start..next_bound];
        if let Some(header_end) = find_subsequence(part_bytes, b"\r\n\r\n") {
            let header_str = String::from_utf8_lossy(&part_bytes[..header_end]);
            let mut content = &part_bytes[header_end + 4..];
            if content.ends_with(b"\r\n") {
                content = &content[..content.len() - 2];
            }

            if let Some(disp) = header_str.lines().find(|l| l.to_lowercase().starts_with("content-disposition")) {
                let name = extract_attr(disp, "name");
                let filename = extract_attr(disp, "filename");

                if let Some(fname) = filename {
                    if !fname.is_empty() {
                        files.push(UploadedFile {
                            filename: fname,
                            data: content.to_vec(),
                        });
                    }
                } else if let Some(n) = name {
                    fields.insert(n, String::from_utf8_lossy(content).trim().to_string());
                }
            }
        }
        start_idx = next_bound;
    }
    (files, fields)
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

    let last_converted_files: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

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

        // Endpoint: Start conversion of existing disk files
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

                    let results: Vec<ConversionResult> = req.files.par_iter().map(|file_str| {
                        let src_path = Path::new(file_str);
                        let stem = src_path.file_stem().unwrap_or_default().to_string_lossy();
                        let parent = src_path.parent().unwrap_or(Path::new("."));
                        let out_dir = parent.join(&cfg.output_subfolder);
                        let out_path = out_dir.join(format!("{}.jpg", stem));

                        convert_single_dng(src_path, &out_path, &cfg)
                    }).collect();

                    if let Ok(mut list) = last_converted_files.lock() {
                        list.clear();
                        for r in &results {
                            if r.success {
                                list.push(PathBuf::from(&r.output_path));
                            }
                        }
                    }

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

        // Endpoint: Browser Drag-and-Drop file upload & conversion
        if method == Method::Post && url.starts_with("/api/jobs/upload") {
            let mut boundary = Vec::new();
            for h in request.headers() {
                if h.field.as_str().as_str().eq_ignore_ascii_case("content-type") {
                    let val = h.value.as_str();
                    if let Some(pos) = val.find("boundary=") {
                        boundary = val[pos + 9..].trim().as_bytes().to_vec();
                    }
                }
            }

            let mut body_bytes = Vec::new();
            let _ = request.as_reader().read_to_end(&mut body_bytes);

            if !boundary.is_empty() && !body_bytes.is_empty() {
                let (uploaded_files, fields) = parse_multipart(&body_bytes, &boundary);

                let quality: u8 = fields.get("quality").and_then(|v| v.parse().ok()).unwrap_or(98);
                let color_prof = fields.get("color_profile").cloned().unwrap_or_else(|| "balanced".to_string());
                let auto_orient: bool = fields.get("auto_orient").map(|v| v != "false").unwrap_or(true);

                let cfg = ConversionConfig {
                    quality: quality.clamp(1, 100),
                    color_profile: ColorProfile::from_str_loose(&color_prof),
                    auto_orient,
                    output_subfolder: "Converted_JPEG".to_string(),
                    allow_magick_fallback: true,
                };

                let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
                let upload_batch_dir = PathBuf::from("workspace_uploads").join(format!("batch_{}", ts));
                let _ = fs::create_dir_all(&upload_batch_dir);

                let mut saved_paths = Vec::new();
                for uf in uploaded_files {
                    if uf.filename.to_lowercase().ends_with(".dng") {
                        let save_path = upload_batch_dir.join(&uf.filename);
                        if let Ok(mut f) = File::create(&save_path) {
                            let _ = f.write_all(&uf.data);
                            saved_paths.push(save_path);
                        }
                    }
                }

                let results: Vec<ConversionResult> = saved_paths.par_iter().map(|src_path| {
                    let stem = src_path.file_stem().unwrap_or_default().to_string_lossy();
                    let out_dir = upload_batch_dir.join(&cfg.output_subfolder);
                    let out_path = out_dir.join(format!("{}.jpg", stem));

                    convert_single_dng(src_path, &out_path, &cfg)
                }).collect();

                if let Ok(mut list) = last_converted_files.lock() {
                    list.clear();
                    for r in &results {
                        if r.success {
                            list.push(PathBuf::from(&r.output_path));
                        }
                    }
                }

                let resp_json = serde_json::json!({
                    "job_id": format!("upload_{}", ts),
                    "total_files": results.len(),
                    "results": results,
                });
                let resp = Response::from_string(resp_json.to_string())
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                let _ = request.respond(resp);
                continue;
            } else {
                let resp = Response::from_string(r#"{"detail":"Invalid multipart payload or missing boundary"}"#)
                    .with_status_code(StatusCode(400))
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                let _ = request.respond(resp);
                continue;
            }
        }

        // Endpoint: Download all as ZIP
        if method == Method::Get && url.starts_with("/api/download-zip") {
            let files_to_zip: Vec<PathBuf> = if let Ok(list) = last_converted_files.lock() {
                list.clone()
            } else {
                Vec::new()
            };

            let mut zip_buffer = Cursor::new(Vec::new());
            {
                let mut zip = ZipWriter::new(&mut zip_buffer);
                let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

                for fpath in &files_to_zip {
                    if fpath.is_file() {
                        let name = fpath.file_name().unwrap_or_default().to_string_lossy();
                        if let Ok(mut src_f) = File::open(fpath) {
                            let mut data = Vec::new();
                            if src_f.read_to_end(&mut data).is_ok() {
                                let _ = zip.start_file(name, options);
                                let _ = zip.write_all(&data);
                            }
                        }
                    }
                }
                let _ = zip.finish();
            }

            let resp_data = zip_buffer.into_inner();
            let resp = Response::from_data(resp_data)
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/zip"[..]).unwrap())
                .with_header(Header::from_bytes(&b"Content-Disposition"[..], &b"attachment; filename=\"Converted_JPEG.zip\""[..]).unwrap());
            let _ = request.respond(resp);
            continue;
        }

        // Endpoint: Image Preview
        if method == Method::Get && url.starts_with("/api/preview") {
            if let Some(query) = url.split('?').nth(1) {
                let params: Vec<(&str, &str)> = query.split('&').filter_map(|p| {
                    let mut parts = p.split('=');
                    Some((parts.next()?, parts.next()?))
                }).collect();

                let is_download = params.iter().any(|(k, v)| *k == "download" && *v == "true");

                if let Some((_, raw_path)) = params.iter().find(|(k, _)| *k == "path") {
                    let decoded_path = raw_path.replace("%2F", "/").replace("%5C", "\\").replace("%20", " ");
                    let path = Path::new(&decoded_path);
                    if path.is_file() {
                        if let Ok(mut f) = File::open(path) {
                            let mut buf = Vec::new();
                            if f.read_to_end(&mut buf).is_ok() {
                                let mut resp = Response::from_data(buf)
                                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"image/jpeg"[..]).unwrap());
                                if is_download {
                                    let fname = path.file_name().unwrap_or_default().to_string_lossy();
                                    resp.add_header(Header::from_bytes(
                                        &b"Content-Disposition"[..],
                                        format!("attachment; filename=\"{}\"", fname).as_bytes()
                                    ).unwrap());
                                }
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
        let _ = request.respond(
            Response::from_string(r#"{"error":"Not Found"}"#)
                .with_status_code(StatusCode(404))
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
        );
    }
}