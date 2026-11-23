# Changelog

---

## [0.7.0] — 2026-06-02 to 2026-10-07

**Release name:** 0.7  
**Baseline (tag `0.6`):** `caa4e17c4b078836cefd7137fb0970e485a929bc` — *Merge pull request #8 from knivier/1.6*, 2026-06-02  
**Release tip:** `998ce846832a1f9c0199bd390e292a612e064547` — *Allowed proper build systems on windows*, 2026-10-07

Release 0.7 is the jump from a C-centric GRUB kernel with a Python builder under `src/` to a **layered x86-64 kernel** with a **native UEFI loader**, a **Rust driver network**, **real block/FAT32 storage**, and a **unified `scripts/core.sh` toolchain**. Tag 0.6 was VMM identity mapping, async IRQ1 keyboard, and timer bring-up (PR #8). Everything below is new since that tag.

---

### Highlights

- **Native UEFI boot** (`BOOTX64.EFI` + GOP framebuffer) so no-CSM machines boot without GRUB/CSM; GRUB Multiboot2 ISO path remains.
- **Unified build** under `scripts/core.sh`; all artifacts in `target/`; Fedora / Linux / macOS / Windows(WSL) wrappers and GUIs.
- **Rust crate `popcorn_kernel`**: driver registry, catalog, screen/kbd/serial backends, PCI, xHCI, virtio-blk, NVMe, USB MSC, FAT32, and all pops.
- **Layered C core**: shell, IDT, IRQ table, syscalls, and init split out of a thinned `kernel.c`.
- **Disk install on USB** with **master-locked NVMe writes**; QEMU auto-install is feature-gated and never flashed as `popcorn-uefi.img`.
- **PIC 2.1 cleanup** (correct ICW3 cascade, `io_wait`, slave-before-master EOI, spurious IRQ7/15).
- **Shell dash commands**: `cl -gettime`, `disk -install|-wipe|-master`, `mem`, `cpu`, `drive`, `catalog`, …
- **Windows WSL** builds a hardware-safe image and runs QEMU on `target/` files only (never `PhysicalDrive`).

---

### Boot, UEFI, and framebuffer

- Native UEFI loader `src/uefi/bootx64.c` → `target/BOOTX64.EFI`; flash image `target/popcorn-uefi.img` (FAT32 `EFI/BOOT/BOOTX64.EFI` + `BOOT/KERNEL`).
- GOP framebuffer console on machines without CSM (`boot_fb.h`, 8×16 font); staged boot splash / init.
- UEFI ConIn keyboard fallback after `ExitBootServices` (`uefi_input.c`).
- Loader loads the kernel **only from the volume that held `BOOTX64.EFI`** (does not scan every FAT disk for `\boot\kernel`).
- Loader refuses a kernel that still contains the QEMU auto-install selftest.
- Post-shell reboot path fixed; Multiboot2 memory-upper handling for 64-bit values.
- QEMU video uses virtio-vga/ramfb for GOP; WSL defaults to VNC (`127.0.0.1:5900`) with optional TigerVNC auto-launch.

---

### Build system, QEMU, and CI

- Replaced `src/buildmon.py` / `src/fed-build.sh` / `src/mac-build.sh` with `scripts/core.sh` + `scripts/lib/*` + `scripts/popcorn_build/`.
- Commands: `all`, `uefi`, `img`, `iso`, `iso-uefi`, `run-uefi`, `test-uefi`, `test-pf`, `test-install`, `clean`.
- `run-uefi` rebuilds when sources are newer than `target/kernel` so QEMU does not boot a stale image.
- QEMU guest disks are **regular files under `target/`** (virtio + USB MSC scratch + emulated NVMe). Host `PhysicalDrive`, `/dev/sd*`, `/dev/nvme*` are refused.
- `target/UNSAFE/` holds QEMU-only images; **do not flash**. The only flashable file is `target/popcorn-uefi.img` from `./scripts/core.sh all` (no `POPCORN_TEST_INSTALL`).
- `test-install` is QEMU-only (`-DPOPCORN_TEST_INSTALL` + Rust feature `qemu-install-selftest`), writes `target/UNSAFE/UNSAFE-*.img`, then rebuilds a safe `popcorn-uefi.img`. Blocked from the Windows GUI / `win.ps1`.
- Extra OVMF code/vars search paths; `core.sh` executable bit; GitHub Actions `test-uefi` smoke (UEFI + GRUB + `#PF` dump).
- Windows: `scripts/win.ps1`, `gui-win.py`, `run-uefi-windows.cmd`. WSL bash gets LF scripts (fixes `set: pipefail` on CRLF), unsets `POPCORN_CFLAGS`, never attaches the Windows disk.

---

### Kernel architecture (C / asm)

- Split `kernel.c` into `shell.c`, `idt.c`, `irq.c`, `kbd.c`, `device.c`, `exception.c`, `phase2_selftest.c`, thinned `kmain`.
- Central IRQ table: `irq_register` / `irq_enable` / `irq_disable`; unexpected/spurious accounting.
- Scheduler: wait queues, `SYS_SLEEP` / `SYS_YIELD`, IRQ wake, bootstrap guard, shared task pool.
- Device fd bridge: `SYS_OPEN` / `READ` / `WRITE` / `IOCTL` go through `dev_*` (Phase 2).
- Diagnosable exceptions: `#PF` / `#GP` / `#DF` dump CR2 / error / RIP on COM1, then halt; `test-pf` QEMU check.
- PMM bitmap tracks up to 16 GiB; QEMU RAM 4 GiB; process PML4 clones kernel upper half; 2 MiB PDE splitter for 4K overlays (`vmm_map_4k`).
- `sys_fork` memory copy fixes; syscall registration cleaned (real fd/dev/time/heap/cwd vs stubs).
- Timer: UEFI poll mode vs GRUB IRQ PIT; `irq_quiet` so USB BOT does not run the scheduler mid-transfer.

---

### PIC (roadmap-UTD §2.1)

- Master ICW3 `0x04`, slave ICW3 `0x02` (cascade IRQ2).
- `io_wait()` between PIC init commands.
- Unmask master IRQ2 when enabling IRQ8–15.
- Slave-before-master EOI; spurious IRQ7 / IRQ15 handling.

---

### Rust driver network, catalog, and pops

- `no_std` crate with `build-std`, large code model, panic → halt.
- Registry: `device` / `driver` / `class` / `dma` / `irq` / `io`; in-kernel **catalog** (`catalog` shell command).
- Backends: VGA `/dev/tty0`, GOP `/dev/fb0`, serial, null, zero, PIT clock, **CMOS RTC**, meminfo, cpuinfo, kbd.
- Bus: PCI multi-bus walk (devices behind PCIe bridges).
- USB: xHCI + USB MSC BOT.
- Block: ramdisk, virtio-blk (multi-device), NVMe, USB MSC.
- All pops moved to Rust (`pops/`): Shimjapii, Spinner, Uptime, Memory, CPU, Sysinfo, Dolphin. C `filesystem_pop` **removed** (replaced by FAT32 on a selected disk).
- `console_ffi` for pops (color print, `CursorGuard`); C console remains the UX client of Rust screen backends.
- Keyboard: Rust `/dev/kbd`; C PS/2 poll/queue removed; shift / caps lock; `drive -load kbd`.

---

### Storage, FAT32, and install safety

- FAT32 on the selected block device (OEM `POPCORN `, UEFI layout). `ls` / `write` / `read` / `mkdir` / `go` talk to that volume — not the old in-memory FS pop.
- Shell: `disk -list|-use|-info|-read|-write|-wipe|-install|-master`.
- **`disk install <name> YES`**: stage `BOOTX64.EFI` + `KERNEL` from a USB/virtio boot volume into RAM, format the **target**, write the UEFI layout. Staging **never reads Internal/NVMe** (leftover Popcorn on NVMe must not poison a USB).
- **`disk wipe`**: FAT32 format without copying boot files.
- **NVMe/internal writes stay off** until `disk master <name> YES` (this boot only). Install/wipe/selftest do not flip that gate. NVMe `WRITE_ENABLED` starts false.
- Boot does **not** auto-install on hardware. Selftest requires compile-time flag **and** CPUID hypervisor **and** QEMU fw_cfg signature `"QEMU"`.
- Flash kernel/img are grepped so `rust_test_disk_install` / `install-selftest` cannot land in `popcorn-uefi.img`.
- Hardware soak: storage retention and USB install on real machines; PIC + xHCI quiet sections for BOT transfers.

---

### Shell and clock

- Dash-style subsets: `cl -help|-gettime`, `mem -map|-use|-stats`, `cpu -info|-hz`, `disk -…`, `drive -…`, `dev -list`, `catalog -list`, `mon -…`, `wrap -on|-off`.
- `cl -gettime` reads CMOS RTC (local date/time) via the Rust clock backend.
- `uptime` / `timer` still report PIT ticks.

---

### Documentation and process

- `readme.md` retargeted to v0.7 and `scripts/core.sh`.
- `roadmap.md` Phase 1–3 coding gates; language-split policy (new drivers in Rust).
- `roadmap-UTD.md` added (hardware/ABI checklist; PIC 2.1 and RTC marked done).
- `pop.md` updated for Rust pops.

---

### Merges included in this period

| Merge | Subject |
|------|---------|
| `a687b08` | Merge branch `1.6` — build script cleanup and repo structure tidy-up |
| `bc425a3` | Merge pull request **#10** from `knivier/rust-testing` |
| `2abe458` | Merge `rust-testing`: include new roadmap-aigen |
| `e944ef4` | Merge pull request **#11** from `knivier/phaser-01-9` |

---

### Full commit appendix (chronological)

*Range:* `caa4e17c4b078836cefd7137fb0970e485a929bc..HEAD` (exclusive of tag `0.6`; oldest → newest)

| Commit | Date | Author | Subject |
|--------|------|--------|---------|
| `d25160c` | 2026-06-02 | Knivier | massive cleanup |
| `a687b08` | 2026-06-02 | Knivier | Merge branch '1.6' — build script cleanup and repo structure tidy-up |
| `cb97efe` | 2026-06-03 | Knivier | Add UEFI native boot path and fix framebuffer console on no-CSM hardware. |
| `03f5462` | 2026-06-03 | Knivier | Fix UEFI post-shell reboot and unify build under core.sh |
| `daf7f32` | 2026-09-22 | Knivier | Add UEFI input handling to build process and improve memory copy in sys_fork |
| `49fabef` | 2026-10-02 | Knivier | rmp updates |
| `41ffe39` | 2026-10-02 | Knivier | Cleanup |
| `d528b6d` | 2026-10-02 | Knivier | IRQ table, #PF tests, scheduler sleep/wake, device syscalls, boot stability |
| `37397c3` | 2026-10-03 | Knivier | Test rust script, autolaunch tigervnc on windows |
| `70eb444` | 2026-10-03 | Knivier | Cleanups en masse, phase 2 goes thru sys_ioctl, cl runs single step, dead code removal |
| `6acad5f` | 2026-10-03 | Knivier | massive reformation of popcorn into a significantly more layered monolithic kernel. Split Shell, ekbyard, and IDT apart and thnned the kernel.c file. |
| `117ea14` | 2026-10-03 | Knivier | scrollback with better bootscreen and proper resolution systems |
| `0dcf8bb` | 2026-10-03 | Knivier | Pop modules converted into Rust for many exmaplers with callbacks for pop modules calling drivers. |
| `47e6ab1` | 2026-10-03 | Knivier | Refactor keyboard handling to utilize Rust-based `/dev/kbd` device operations. |
| `50e1d8f` | 2026-10-03 | Knivier | Update roadmap and system call implementation; refine console interactions and Rust integration. |
| `3c69087` | 2026-10-03 | Knivier | Enhance memory management and virtual memory system; QEMU 4 GiB; PMM ≤16 GiB; process PML4; PDE splitter. |
| `e3278d6` | 2026-10-03 | Knivier | Disk management, multi-virtio, keyboard shift/caps, QEMU data disks. |
| `86f18d3` | 2026-10-03 | Knivier | Replace in-memory filesystem pop with Rust FAT32 on selected block devices. |
| `08218af` | 2026-10-05 | Knivier | Multiboot2 64-bit memory upper; PCI multi-bus; FAT32/USB MSC error output. |
| `216661c` | 2026-10-05 | Knivier | Change script permissions for core.sh to make it executable |
| `124bd4b` | 2026-10-05 | Knivier | Add additional OVMF code and vars paths to find_edk_code and ensure_ovmf_vars functions |
| `7dad8b4` | 2026-10-05 | Knivier | Safety bound check for test case |
| `bc425a3` | 2026-10-05 | Knivier | Merge pull request #10 from knivier/rust-testing |
| `b8f783d` | 2026-10-05 | Knivier | new roadmap-aigen |
| `2abe458` | 2026-10-06 | Knivier | Merge rust-testing: include new roadmap-aigen |
| `88cf1ce` | 2026-10-06 | Knivier | Working-storage retention on real hardware |
| `7354507` | 2026-10-06 | Knivier | Working safeguarded storage retention with installation available for popcorn to be written onto drives and booted from there from an installation copy with master locks |
| `9a61fb0` | 2026-10-06 | Knivier | PIC Cleanup complete, posted on readme |
| `e944ef4` | 2026-10-07 | Knivier | Merge pull request #11 from knivier/phaser-01-9 |
| `afb7e89` | 2026-10-07 | Knivier | RTC rdmp upd |
| `9ef5a25` | 2026-10-07 | Knivier | RTC/CMOS data pulling on driver with cl -subset commands, organised subset shell commands with dash type command architecture |
| `c75c1cc` | 2026-10-07 | Knivier | Update readme.md |
| `5ccd04d` | 2026-10-07 | Knivier | rename 0.7 |
| `6946c6c` | 2026-10-07 | Knivier | Update roadmap-UTD.md: define constant version number for popcorn and mark RTC data fetching as complete |
| `b390c6a` | 2026-10-07 | Knivier | Update roadmap-UTD.md: revise suggested names for ABI pipelines and enhance clarity on access levels |
| `5dd94a2` | 2026-10-07 | Knivier | optimisations and cleanups |
| `998ce84` | 2026-10-07 | Knivier | Allowed proper build systems on windows |

---

### Notes for readers upgrading from 0.6

- **Flash only** `target/popcorn-uefi.img` from `./scripts/core.sh all` (or `scripts/win.ps1 all`). Never flash `target/UNSAFE/*`. Never run `test-install` and then Etcher that image.
- Boot must show the UEFI loader and a **shell with no automatic install**. USB install is `disk install usb0 YES`. NVMe writes require `disk master nvme0 YES` first.
- In the shell, clock is `cl -gettime` (not a `clock` command). Also `uptime` and `timer`.
- This changelog is from **git history** `0.6..HEAD` (37 commits, ~173 files, +21k/−7k lines). Commit subjects are copied as recorded.

---

## [0.5.0] — 2025-10-17 to 2026-03-10

**Release name:** 0.5  
**Baseline commit:** `7f9ceb9c7d21770b0b4407ed829f61a03191d3fa`, 2025-10-17
**Release tip (at time of writing):** `1cdcf3909055e16355edf558a5a06bc6a368b95e` — *Merge pull request #7 from knivier/finaladdits*, 2026-03-10

Release 0.5 spans the evolution of Popcorn from **heavy console and filesystem work** through the **x86-64 bring-up**, **new build/ISO/QEMU workflow**, **preemption and scheduling**, **system call surface expansion**, and **stability/security hardening**.

The best thing Popcorn now has to offer, is any user with an x64 PC can now boot into the ISO and try Popcorn. To try, load the ISO onto a USB stick (recommended to use Ventoy) and make sure secure boot is off. Have fun! General system compatibility is likely, but edge cases haven't been tested. This update spans a year of work, sweat, and tears, and h as been the epitimy of updates.

---

### Highlights

- **Console subsystem matured**: rendering, formatting, input handling, scrollback, history, tab completion, and a long series of correctness fixes (including a keyboard double-processing bug).
- **Filesystem and “Dolphin” UX**: richer file operations, editor behavior, navigation, validation, and filesystem-related error handling; merged via **PR #5** (`filesys_improvements`).
- **Official x64 conversion path**: kernel packaged as an **ISO** and run under **QEMU** separately; **Python-based builder** oriented toward Fedora workflows.
- **Hardware introspection**: extended memory map handling, CPU frequency detection, CPU detection, CPUID support, multiboot info persistence, and `sysinfo`-style commands.
- **Kernel architecture growth**: interrupt-driven design, timers, memory management, scheduler, syscall interface, initialization split, and a working **task switch**.
- **Syscall expansion**: `fork`, `wait`, `mmap`, `munmap`, `stat`, `ioctl` implementations with integration and error-handling emphasis.
- **Engineering quality**: refactors for typos/debug clutter/shadowing, shared constants/utilities consolidation, bug fixes, and security-oriented fixes.

---

### Console, terminal UX, and input

- Major console display fixes and formatting improvements.
- Fixed keyboard interrupt handling where the handler could **double-process** input.
- Standardized console usage across pops via shared console utilities; removed unused functions and updated documentation.
- Scrollback structures, command history, and tab autocomplete foundations.
- Scrollback buffer logic fixes, cursor positioning fixes, standardized error messages, arrow-key constraints, and related command hardening in the shell-facing paths.
- README updates; removal of obsolete “trymake” references.

---

### Filesystem, Dolphin editor, and related commands

- VGA access fixes; operand bounds checking; broader error handling and validation.
- Double buffering work; `ls` / `rm` and related command behavior improvements; search/copy flows; root filesystem structures with file structs.
- Dedicated filesystem improvements branch merged: **PR #5** — *filesys_improvements*.
- Dolphin text editor: text file handling and navigation improvements; Python builder upgrades alongside Dolphin work.

---

### Boot, hardware detection, and multiboot

- Extended memory map handling and related boot-time information handling.
- CPU frequency detection, CPU detection, CPUID support, and additional hardware interaction surfaces.
- Multiboot detection attempts and fixes; multiboot information saved for later use by kernel subsystems.
- `sysinfo`-style reporting and related command surfacing.

---

### Build system, ISO, QEMU, and developer tooling

- **x64 “official conversion”**: kernel linked into an **ISO** and executed under **QEMU** as a separate step from the build.
- **Custom Python builder** oriented for Fedora-based workflows.
- Builder updated to avoid a prior ISO-centric flow and include necessary files explicitly.
- `build.sh` structure aligned with the building system used by `buildmon.py`.
- Build error cleanup commits; serial debugging cleanup; Python builder improvements to show serial output.
- Merge integration work around serial corruption recovery (`corruptundo` → `x64try` attempt) culminating in error fixes and **PR #6** merge.

---

### Kernel architecture: interrupts, memory, time, scheduling

- Interrupt-driven architecture; memory management system; timer interrupts; scheduler foundations.
- Split initialization paths to reduce monolithic startup.
- System call interface scaffolding and iterative integration.
- Successful internal restructure milestone.
- **Functioning task switch** milestone (preemption/scheduling bring-up).

---

### Syscalls and OS interfaces

- Expanded syscall surface with substantive implementations for:

  - `fork`
  - `wait`
  - `mmap` / `munmap`
  - `stat`
  - `ioctl`

- Emphasis on error handling and subsystem integration in syscall implementations.

---

### Reliability, security, and maintenance

- Refactor pass: typos, debug clutter removal, shadowing fixes, constants, utility consolidation.
- Bug fixes and security-oriented fixes (dedicated “bug and security fixes” commit).
- Stabilization “point check” commit and later build follow-ups (including `f43 build work`).
- Final integration merge: **PR #7** — *finaladdits*.

---

### Merges included in this period

| Merge | Subject |
|------|---------|
| `72d6f24` | Merge pull request **#5** from `knivier/filesys_improvements` |
| `8b6159e` | Merge pull request **#6** from `knivier/x64try` |
| `1cdcf39` | Merge pull request **#7** from `knivier/finaladdits` |

---

### Full commit appendix (chronological)

*Range:* `7f9ceb9c7d21770b0b4407ed829f61a03191d3fa..HEAD` (exclusive of baseline; oldest → newest)

| Commit | Date | Author | Subject |
|--------|------|--------|---------|
| `6479185` | 2025-10-17 | Knivier | Massive console display fixes |
| `f4d85dd` | 2025-10-17 | Knivier | Beautiful format, all fixed and new console output 100% working. Keyboard interrupt handler was double processing, now fixed. |
| `400f164` | 2025-10-17 | Knivier | Removed unused functions, common utils, all pops use the shared console funcs, updated all docs |
| `98824bb` | 2025-10-18 | Knivier | VGA acess fixes, bounds checking for file operands, error handling, version updates, input validation, double buffering, ls & rm commands, search, copy, rootsys w file structs |
| `a70d1ba` | 2025-10-18 | Knivier | Error handling |
| `72d6f24` | 2025-10-18 | Knivier | Merge pull request #5 from knivier/filesys_improvements |
| `40ca886` | 2025-10-18 | Knivier | x64 official conversion, Kernel is now linked into an ISO which is ran through QEMU separately. A custom build system has been created through Python for Fedora. |
| `8e9610c` | 2025-10-18 | Knivier | MD updates |
| `98f84b0` | 2025-10-19 | Knivier | Extended memmap, frequency detection, CPU detection, sysinfo commands, multiboot info saved, CPUID support added, hardware interaction |
| `30b91bb` | 2025-10-19 | Knivier | Attempted multiboot detection fixing |
| `c5838c0` | 2025-10-19 | Knivier | updated builder to not use iso system and include all files |
| `0bad2e9` | 2025-10-19 | Knivier | Added scrollback byffer structures, command history, tab autocomplete |
| `5c63e06` | 2025-10-19 | Knivier | Dolphin text editor, now handles text files |
| `d981007` | 2025-10-19 | Knivier | Dolphin navigation fixes, python builder upgrade |
| `13471ef` | 2025-10-19 | Knivier | Removed unused methods, bound checking for all commands in kernel.c, scrollback buffer logic fixed, cursor positioning fixed, standardized error messages, arrow key constraints, file size truncation wraning |
| `77d47fa` | 2025-10-19 | Knivier | trymake references removed |
| `512aaba` | 2025-10-19 | Knivier | updated readme |
| `16c490d` | 2025-10-20 | Knivier | Interrupt-driven architecture, memory management system, timer interrupts, and schedular have all been built |
| `e1da8b8` | 2025-10-20 | Knivier | separate initializement system |
| `2f10f5f` | 2025-10-20 | Knivier | System call interface |
| `a1b3641` | 2025-10-21 | Knivier | Successful restructure |
| `1751d1e` | 2025-10-21 | Knivier | Fixed the struture of build.sh to match the building system in buildmon.py |
| `dc2b8ee` | 2025-10-21 | Knivier | Rmd builderrors |
| `468fdda` | 2025-10-21 | Knivier | rmd build errors |
| `713da4b` | 2025-10-21 | Knivier | Functioning taskswitch! |
| `f0a5c9a` | 2025-10-21 | Knivier | Cleaned files and serial debugging |
| `6341a92` | 2025-10-21 | Knivier | Builder fixed to show serial output on py |
| `bb732e4` | 2025-10-21 | Knivier | Cleaned serial, fixed corruption |
| `769e440` | 2025-10-21 | Knivier | attempting merge branch 'corruptundo' into x64try |
| `636f2b8` | 2025-10-21 | Knivier | Fixed all errors |
| `8b6159e` | 2025-10-21 | Knivier | Merge pull request #6 from knivier/x64try |
| `664a4e4` | 2025-11-11 | Knivier | Refactor: fix typos, remove debug clutter, eliminate shadowing, add constants, consolidate utilities |
| `b0caad8` | 2025-11-11 | Knivier | Add advanced implementations for fork, wait, mmap, munmap, stat, and ioctl syscalls with proper error handling and subsystem integration. |
| `6db96ff` | 2025-11-14 | Knivier | bug and security fixes |
| `7c14dfb` | 2025-11-14 | Knivier | point check |
| `bbb4fda` | 2026-01-15 | Knivier | f43 build work |
| `1cdcf39` | 2026-03-10 | Knivier | Merge pull request #7 from knivier/finaladdits |

---

### Notes for readers upgrading to 0.5

- This changelog is generated from **git history** and commit subjects; it intentionally reflects what was recorded at commit time (including typos present in original messages).
- If you need a stricter “semantic versioning” changelog (Added/Changed/Fixed with issue links), tell me your preferred link format (GitHub compare URL, issue prefixes, etc.) and I can reshape this document accordingly.
