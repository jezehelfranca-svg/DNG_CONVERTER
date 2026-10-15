@echo off
title Building Rust DNG Converter Standalone Binary
cd /d "%~dp0\rust"
echo ============================================================================
echo   Building Rust DNG Converter (Optimized Release .exe)
echo ============================================================================
cargo build --release
if %ERRORLEVEL% equ 0 (
    echo.
    echo ============================================================================
    echo   BUILD SUCCESSFUL!
    echo   Standalone executable created at:
    echo   rust\target\release\dng_converter.exe
    echo ============================================================================
) else (
    echo [ERROR] Cargo build failed.
)
pause