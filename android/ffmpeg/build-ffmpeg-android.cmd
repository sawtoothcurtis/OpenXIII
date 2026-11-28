@echo off
rem Builds libffmpeg.so for the OpenXIII APK (cutscenes on the phone).
rem Takes a few minutes. Output in <tools>\ffmpeg\arm64-v8a, where <tools> is OXIII_TOOLS
rem (default: the OpenXIII-android-tools folder next to the repository).
rem Runs in MSYS2 UCRT64 (its make and gcc), like the PC ffmpeg.exe.
rem MSYS2 folder: MSYS2_ROOT, default C:\msys64 (the MSYS2 installer's default).
rem Work folder (sources, objects): OXIII_FFMPEG_WORK, default: the OpenXIII-build\ffmpeg-android
rem folder next to the repository.

cd /d "%~dp0"
if "%MSYS2_ROOT%"=="" set MSYS2_ROOT=C:\msys64
set MSYS2=%MSYS2_ROOT%\msys2_shell.cmd
if not exist "%MSYS2%" (
    echo MSYS2 not found in %MSYS2_ROOT% ^(set MSYS2_ROOT^).
    pause
    exit /b 1
)
call "%MSYS2%" -ucrt64 -defterm -no-start -here -c "bash ./build-ffmpeg-android.sh"

echo.
echo Done. You can close this window.
pause
