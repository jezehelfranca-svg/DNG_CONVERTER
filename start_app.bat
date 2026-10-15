@echo off
title DNG to High-Quality JPEG Web Studio
cd /d "%~dp0"
echo ============================================================================
echo   DNG to High-Quality JPEG Studio Web App
echo   URL: http://127.0.0.1:8080
echo ============================================================================
start "" "http://127.0.0.1:8080"
python app.py
pause