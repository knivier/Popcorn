@echo off
title Popcorn QEMU (VNC)
cd /d "%~dp0.."
echo.
echo  Popcorn is starting under QEMU with VNC.
echo  When you see "icdPRteKLM", open a VNC viewer and connect to:
echo.
echo      127.0.0.1:5900
echo.
echo  Recommended viewers (pick one):
echo    - TigerVNC / TightVNC / RealVNC Viewer
echo.
echo  Leave this window open while you use Popcorn.
echo.
wsl -e bash -lc "cd /mnt/c/Users/Knivier/Documents/Code/popcorn && export POPCORN_QEMU_DISPLAY=vnc && ./scripts/core.sh run-uefi"
pause
