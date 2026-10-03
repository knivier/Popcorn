@echo off
title Popcorn QEMU (VNC)
cd /d "%~dp0.."
echo.
echo  Popcorn UEFI via WSL + TigerVNC
echo  VNC: 127.0.0.1:5900
echo.

REM Kill any leftover guest so the port is free.
wsl -e bash -lc "killall qemu-system-x86_64 2>/dev/null || true"

REM Start QEMU in a new window (keeps serial output visible).
start "Popcorn QEMU" wsl -e bash -lc "cd /mnt/c/Users/Knivier/Documents/Code/popcorn && export POPCORN_QEMU_DISPLAY=vnc && ./scripts/core.sh run-uefi"

echo  Waiting for VNC on 127.0.0.1:5900 ...
powershell -NoProfile -Command ^
  "$ok=$false; for ($i=0; $i -lt 90; $i++) { try { $c=New-Object Net.Sockets.TcpClient; $c.Connect('127.0.0.1',5900); $c.Close(); $ok=$true; break } catch { Start-Sleep -Milliseconds 250 } }; if (-not $ok) { Write-Host 'VNC not ready — connect manually to 127.0.0.1:5900'; exit 1 }"

set "VNC="
if exist "%ProgramFiles%\TigerVNC\vncviewer.exe" set "VNC=%ProgramFiles%\TigerVNC\vncviewer.exe"
if not defined VNC if exist "%ProgramFiles(x86)%\TigerVNC\vncviewer.exe" set "VNC=%ProgramFiles(x86)%\TigerVNC\vncviewer.exe"
if not defined VNC if exist "%LOCALAPPDATA%\Programs\TigerVNC\vncviewer.exe" set "VNC=%LOCALAPPDATA%\Programs\TigerVNC\vncviewer.exe"
if not defined VNC if exist "%ProgramFiles%\TightVNC\tvnviewer.exe" set "VNC=%ProgramFiles%\TightVNC\tvnviewer.exe"

if defined VNC (
  echo  Opening %VNC% -^> 127.0.0.1:5900
  start "" "%VNC%" 127.0.0.1:5900
) else (
  echo  No TigerVNC/TightVNC found. Connect manually to 127.0.0.1:5900
  echo  Or set POPCORN_VNC_VIEWER to your viewer .exe
)

echo.
echo  Leave the QEMU window open while using Popcorn.
pause
