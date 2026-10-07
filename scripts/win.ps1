# Popcorn Windows entry: WSL2 builds a hardware-safe UEFI image and runs QEMU
# against files under target/ only. Host PhysicalDrive / NVMe is never attached.
# Usage:  powershell -File scripts\win.ps1 all
#         powershell -File scripts\win.ps1 run-uefi
param(
    [Parameter(Position = 0)]
    [string]$Command = "help"
)

$ErrorActionPreference = "Stop"
$Repo = Split-Path -Parent $PSScriptRoot

$blocked = @("test-install")
if ($blocked -contains $Command.ToLowerInvariant()) {
    Write-Error @"
Blocked on Windows: $Command
That command builds an auto-install kernel. It must never be flashed.
QEMU on this path only uses regular files under target\ — never the Windows disk.
If you need the QEMU selftest, run it inside WSL: ./scripts/core.sh test-install
"@
    exit 2
}

if (-not (Get-Command wsl -ErrorAction SilentlyContinue)) {
    Write-Error "WSL not found. Install WSL2 and a distro with nasm clang lld qemu-system-x86 ovmf/edk2-ovmf mtools rustup."
    exit 2
}

# WSL bash treats `set -o pipefail\r` as an invalid option. Normalize scripts to LF.
Get-ChildItem -Path (Join-Path $Repo "scripts") -Filter *.sh -Recurse -File | ForEach-Object {
    $bytes = [IO.File]::ReadAllBytes($_.FullName)
    if ($bytes -contains 13) {
        $text = [Text.Encoding]::UTF8.GetString($bytes)
        $utf8 = New-Object Text.UTF8Encoding $false
        [IO.File]::WriteAllText($_.FullName, $text.Replace("`r", ""), $utf8)
        Write-Host "Converted $($_.Name) to LF for WSL bash"
    }
}

$full = [IO.Path]::GetFullPath($Repo)
if ($full -notmatch '^([A-Za-z]):\\(.*)$') {
    Write-Error "Cannot convert repo path to WSL: $full"
    exit 1
}
$wslRoot = "/mnt/$($Matches[1].ToLowerInvariant())/$($Matches[2].Replace('\','/'))"

$safeCmd = $Command -replace '[^a-zA-Z0-9_-]', ''
if ([string]::IsNullOrWhiteSpace($safeCmd)) {
    $safeCmd = "help"
}

$extra = ""
if ($safeCmd -eq "run-uefi") {
    # Rebuild first so QEMU does not boot a stale target/popcorn-uefi.img.
    $extra = "export POPCORN_QEMU_DISPLAY=vnc; bash ./scripts/core.sh all && "
}

# Unset POPCORN_CFLAGS so Windows cannot bake the QEMU auto-install kernel.
$remote = "cd '$wslRoot' || exit 1; unset POPCORN_CFLAGS; export PATH=`"`$HOME/.cargo/bin:`$PATH`"; ${extra}exec bash ./scripts/core.sh $safeCmd"

Write-Host "WSL: hardware-safe UEFI (no auto-install). QEMU disks = target/ files only."
Write-Host "wsl -e bash -lc ..."
& wsl -e bash -lc $remote
exit $LASTEXITCODE
