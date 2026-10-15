@echo off
setlocal EnableDelayedExpansion
chcp 65001 >nul

:: ============================================================================
:: High-Quality DNG to JPEG Batch Converter
:: Uses ImageMagick (7.x+) with 4:4:4 Chroma Subsampling, Auto-Orient & sRGB
:: ============================================================================

set "JPEG_QUALITY=98"
set "OUTPUT_SUBFOLDER=Converted_JPEG"
set "MAGICK_CMD=magick"
set "NO_PAUSE=0"

:: Check if ImageMagick is installed and in PATH
where %MAGICK_CMD% >nul 2>&1
if %ERRORLEVEL% neq 0 (
    if exist "C:\Program Files\ImageMagick-7.1.2-Q16-HDRI\magick.exe" (
        set "MAGICK_CMD="C:\Program Files\ImageMagick-7.1.2-Q16-HDRI\magick.exe""
    ) else (
        echo [ERROR] ImageMagick (magick.exe) was not found in PATH.
        echo Please install ImageMagick or add it to your system PATH.
        pause
        exit /b 1
    )
)

echo ============================================================================
echo   DNG to High-Quality JPEG Batch Converter (Quality: %JPEG_QUALITY%%%, 4:4:4 sRGB)
echo ============================================================================
echo.

set /a TOTAL_SUCCESS=0
set /a TOTAL_FAIL=0

:: Case 1: Arguments passed (Command-line or Drag-and-Drop files/folders)
if "%~1"=="" goto :no_args

:arg_loop
if "%~1"=="" goto :finish
if /i "%~1"=="--no-pause" (
    set "NO_PAUSE=1"
    shift
    goto :arg_loop
)
if exist "%~1\" (
    call :process_folder "%~1"
) else if exist "%~1" (
    call :process_file "%~1"
) else (
    echo [WARN] Path not found: "%~1"
)
shift
goto :arg_loop

:no_args
:: Case 2: No arguments passed -> Check current directory or script directory for .dng files
set "TARGET_DIR=%CD%"
dir /b "%TARGET_DIR%\*.dng" >nul 2>&1
if %ERRORLEVEL% equ 0 (
    call :process_folder "%TARGET_DIR%"
    goto :finish
)

set "SCRIPT_DIR=%~dp0"
if "%SCRIPT_DIR:~-1%"=="\" set "SCRIPT_DIR=%SCRIPT_DIR:~0,-1%"
dir /b "%SCRIPT_DIR%\*.dng" >nul 2>&1
if %ERRORLEVEL% equ 0 (
    call :process_folder "%SCRIPT_DIR%"
    goto :finish
)

:: Case 3: No .dng files in current/script directory -> Prompt user for folder path
echo No .dng files found in current directory:
echo   "%CD%"
echo.
set /p "USER_DIR=Enter or drag-and-drop folder path containing .dng files: "
set "USER_DIR=%USER_DIR:"=%"

if "%USER_DIR%"=="" (
    echo [INFO] No directory specified. Exiting.
    exit /b 0
)

if exist "%USER_DIR%\" (
    call :process_folder "%USER_DIR%"
) else if exist "%USER_DIR%" (
    call :process_file "%USER_DIR%"
) else (
    echo [ERROR] Directory or file does not exist: "%USER_DIR%"
    if "%NO_PAUSE%"=="0" pause
    exit /b 1
)

:finish
echo.
echo ============================================================================
echo   Batch Conversion Complete!
echo   Successfully converted : %TOTAL_SUCCESS% file(s)
if %TOTAL_FAIL% gtr 0 (
    echo   Failed conversions     : %TOTAL_FAIL% file(s)
)
echo ============================================================================
echo.
if "%NO_PAUSE%"=="0" pause
exit /b 0

:: ----------------------------------------------------------------------------
:: Subroutine: Process all .dng files in a folder
:: ----------------------------------------------------------------------------
:process_folder
set "SRC_DIR=%~1"
if "%SRC_DIR:~-1%"=="\" set "SRC_DIR=%SRC_DIR:~0,-1%"
set "OUT_DIR=%SRC_DIR%\%OUTPUT_SUBFOLDER%"

echo [FOLDER] Scanning: "%SRC_DIR%"
dir /b "%SRC_DIR%\*.dng" >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [INFO] No .dng files found in "%SRC_DIR%".
    exit /b 0
)

if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"

for %%F in ("%SRC_DIR%\*.dng") do (
    call :convert_single "%%~fF" "%OUT_DIR%\%%~nF.jpg"
)
exit /b 0

:: ----------------------------------------------------------------------------
:: Subroutine: Process a single dropped .dng file
:: ----------------------------------------------------------------------------
:process_file
set "FILE_PATH=%~f1"
set "FILE_DIR=%~dp1"
if "%FILE_DIR:~-1%"=="\" set "FILE_DIR=%FILE_DIR:~0,-1%"
set "FILE_NAME=%~n1"
set "FILE_EXT=%~x1"

if /i not "%FILE_EXT%"==".dng" (
    echo [SKIP] Not a DNG file: "%~nx1"
    exit /b 0
)

set "OUT_DIR=%FILE_DIR%\%OUTPUT_SUBFOLDER%"
if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"

call :convert_single "%FILE_PATH%" "%OUT_DIR%\%FILE_NAME%.jpg"
exit /b 0

:: ----------------------------------------------------------------------------
:: Subroutine: Run ImageMagick conversion on a single file
:: ----------------------------------------------------------------------------
:convert_single
set "IN_FILE=%~1"
set "OUT_FILE=%~2"

<nul set /p "=  -> Converting %~nx1 ... "
%MAGICK_CMD% "%IN_FILE%[0]" -auto-orient -colorspace sRGB -auto-level -sigmoidal-contrast 3x50%% -modulate 100,112,100 -sampling-factor 4:4:4 -quality %JPEG_QUALITY% "%OUT_FILE%"
if %ERRORLEVEL% equ 0 (
    echo [OK]
    set /a TOTAL_SUCCESS+=1
) else (
    echo [FAILED]
    set /a TOTAL_FAIL+=1
)
exit /b 0