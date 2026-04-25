# Changelog - 0.5 Release Notes
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
