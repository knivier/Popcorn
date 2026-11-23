@echo off
title Popcorn QEMU (VNC)
cd /d "%~dp0.."
echo.
echo  Popcorn UEFI via WSL + TigerVNC
echo  Hardware-safe image only. QEMU uses target\ files — never the Windows disk.
echo  VNC: 127.0.0.1:5900
echo.

REM Stop a leftover Popcorn guest so the port is free (other QEMU VMs are left alone).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0win.ps1" stop >nul 2>&1

REM Start QEMU in a new window (keeps serial output visible).
start "Popcorn QEMU" powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0win.ps1" run-uefi

REM win.ps1 run-uefi rebuilds everything (core.sh all, Rust build-std) before QEMU
REM opens the port, so wait up to 10 minutes rather than a few seconds.
echo  Waiting for VNC on 127.0.0.1:5900 (the build runs first; this can take minutes) ...
powershell -NoProfile -Command ^
  "$deadline=(Get-Date).AddMinutes(10); while ((Get-Date) -lt $deadline) { try { $c=New-Object Net.Sockets.TcpClient; $c.Connect('127.0.0.1',5900); $c.Close(); exit 0 } catch { Start-Sleep -Milliseconds 500 } }; exit 1"
set "VNC_READY=%errorlevel%"

set "VNC="
if defined POPCORN_VNC_VIEWER if exist "%POPCORN_VNC_VIEWER%" set "VNC=%POPCORN_VNC_VIEWER%"
if not defined VNC if exist "%ProgramFiles%\TigerVNC\vncviewer.exe" set "VNC=%ProgramFiles%\TigerVNC\vncviewer.exe"
if not defined VNC if exist "%ProgramFiles(x86)%\TigerVNC\vncviewer.exe" set "VNC=%ProgramFiles(x86)%\TigerVNC\vncviewer.exe"
if not defined VNC if exist "%LOCALAPPDATA%\Programs\TigerVNC\vncviewer.exe" set "VNC=%LOCALAPPDATA%\Programs\TigerVNC\vncviewer.exe"
if not defined VNC if exist "%ProgramFiles%\TightVNC\tvnviewer.exe" set "VNC=%ProgramFiles%\TightVNC\tvnviewer.exe"

if not "%VNC_READY%"=="0" (
  echo  VNC did not come up. Check the "Popcorn QEMU" window for build or QEMU errors.
  echo  Once QEMU is running, connect manually to 127.0.0.1:5900
) else if defined VNC (
  echo  Opening "%VNC%" -^> 127.0.0.1:5900
  start "" "%VNC%" 127.0.0.1:5900
) else (
  echo  No TigerVNC/TightVNC found. Connect manually to 127.0.0.1:5900
  echo  Or set POPCORN_VNC_VIEWER to your viewer .exe
)

echo.
echo  Leave the QEMU window open while using Popcorn.
pause
