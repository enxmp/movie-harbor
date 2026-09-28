@echo off
cd /d "%~dp0"
call npm ci
if errorlevel 1 exit /b 1
call npm run build
if errorlevel 1 exit /b 1
cd src-tauri
cargo build --release --features custom-protocol -j 2
if errorlevel 1 exit /b 1
echo Built: %CD%\target\release\movie-harbor.exe
