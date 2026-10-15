mod color;
mod engine;
mod error;
mod server;

use std::path::{Path, PathBuf};
use clap::Parser;
use rayon::prelude::*;

use crate::color::ColorProfile;
use crate::engine::{convert_single_dng, ConversionConfig};

#[derive(Parser, Debug)]
#[command(name = "dng_converter")]
#[command(author = "jezehelfranca-svg")]
#[command(version = "1.0.0")]
#[command(about = "High-performance standalone DNG to High-Quality JPEG Studio & Batch Converter in pure Rust")]
struct Cli {
    /// Input .dng files or directories to convert in CLI batch mode
    #[arg(value_name = "INPUTS")]
    inputs: Vec<PathBuf>,

    /// Launch embedded Web Studio UI in your default browser
    #[arg(short = 's', long = "serve")]
    serve: bool,

    /// HTTP port for Web Studio server (default: 8080)
    #[arg(short = 'p', long = "port", default_value_t = 8080)]
    port: u16,

    /// JPEG quality from 1 to 100 (default: 98)
    #[arg(short = 'q', long = "quality", default_value_t = 98)]
    quality: u8,

    /// Adaptive Color Optimization profile: balanced, vibrant, portrait, cinematic, none
    #[arg(short = 'c', long = "color-profile", default_value = "balanced")]
    color_profile: String,

    /// Output subfolder name (default: Converted_JPEG)
    #[arg(short = 'o', long = "output-subfolder", default_value = "Converted_JPEG")]
    output_subfolder: String,

    /// Bounded parallel worker threads (default: auto)
    #[arg(short = 't', long = "threads")]
    threads: Option<usize>,

    /// Disable ImageMagick fallback (pure-Rust only)
    #[arg(long = "no-magick-fallback")]
    no_magick_fallback: bool,
}

fn main() {
    let cli = Cli::parse();

    // If --serve requested or no input files provided, start embedded Web Studio!
    if cli.serve || cli.inputs.is_empty() {
        server::run_server(cli.port);
        return;
    }

    // Configure Rayon thread pool if specified
    if let Some(t) = cli.threads {
        let _ = rayon::ThreadPoolBuilder::new().num_threads(t).build_global();
    }

    let config = ConversionConfig {
        quality: cli.quality.clamp(1, 100),
        color_profile: ColorProfile::from_str_loose(&cli.color_profile),
        auto_orient: true,
        output_subfolder: cli.output_subfolder.clone(),
        allow_magick_fallback: !cli.no_magick_fallback,
    };

    let mut files_to_convert: Vec<PathBuf> = Vec::new();
    for input in &cli.inputs {
        if input.is_dir() {
            for entry in walkdir::WalkDir::new(input).into_iter().filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.is_file() {
                    if let Some(ext) = p.extension() {
                        if ext.to_string_lossy().eq_ignore_ascii_case("dng") {
                            files_to_convert.push(p.to_path_buf());
                        }
                    }
                }
            }
        } else if input.is_file() {
            files_to_convert.push(input.clone());
        }
    }

    if files_to_convert.is_empty() {
        println!("[WARN] No .dng files found in specified inputs.");
        return;
    }

    println!("============================================================================");
    println!("  DNG to High-Quality JPEG Batch Converter (Rust Engine)");
    println!("  Found {} file(s) | Quality: {}% | Profile: {}", files_to_convert.len(), config.quality, cli.color_profile);
    println!("============================================================================");

    let start_all = std::time::Instant::now();
    let total = files_to_convert.len();

    let results: Vec<_> = files_to_convert.par_iter().enumerate().map(|(idx, src_path)| {
        let stem = src_path.file_stem().unwrap_or_default().to_string_lossy();
        let parent = src_path.parent().unwrap_or(Path::new("."));
        let out_dir = parent.join(&config.output_subfolder);
        let out_path = out_dir.join(format!("{}.jpg", stem));

        let res = convert_single_dng(src_path, &out_path, &config);
        if res.success {
            println!("  [{}/{}] {} -> {} ({}x{}, {:.2} MB, {:.2}s via {})",
                idx + 1, total, res.filename, res.output_filename, res.width, res.height,
                (res.output_size_bytes as f64) / 1048576.0, res.duration_sec, res.engine_used
            );
        } else {
            println!("  [{}/{}] FAILED {}: {}", idx + 1, total, res.filename, res.error.as_deref().unwrap_or("Unknown error"));
        }
        res
    }).collect();

    let success_count = results.iter().filter(|r| r.success).count();
    let failed_count = total - success_count;
    let elapsed = start_all.elapsed().as_secs_f64();

    println!("============================================================================");
    println!("  Batch Completed in {:.2}s", elapsed);
    println!("  Successfully Converted: {}", success_count);
    if failed_count > 0 {
        println!("  Failed:                 {}", failed_count);
    }
    println!("============================================================================");
}