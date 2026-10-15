@echo off
title DNG Converter Studio (Rust Standalone)
cd /d "%~dp0"
if exist "rust\target\release\dng_converter.exe" (
    start "" "rust\target\release\dng_converter.exe" --serve
) else (
    echo [INFO] Release binary not found. Building it now...
    cd rust
    cargo run --release -- --serve
)