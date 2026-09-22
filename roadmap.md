
# Popcorn — Project Roadmap

## Overview

Popcorn is a modular x86-64 kernel framework for learning operating system development. It ships with a native UEFI loader, GRUB Multiboot2 ISO boot, an interactive in-kernel shell, and a pop-module extension system.

**Current version:** v0.5 (pre-release)

**Roadmap scope:** Everything from the current codebase through a complete driver framework and first-class kernel drivers. Items beyond that (VFS, userspace, networking) are noted only where they depend on drivers.

---

## Maturity Snapshot

| Area | Status |
|------|--------|
| Boot (GRUB + UEFI) | Done |
| Console / shell UX | Done |
| PMM + kmalloc | Done (prototype) |
| VMM (4-level paging) | Partial |
| Scheduler + context switch | Partial |
| Syscalls (21 registered) | Partial (teaching stubs) |
| In-memory filesystem (pop) | Done (not kernel VFS) |
| Driver framework | Not started |
| Block / PCI / storage drivers | Not started |

---

## Phase 0 — Completed (v0.5)

### Boot and build

- [x] x86-64 long-mode entry with identity map and high-half kernel (`src/core/kernel.asm`, `src/link.ld`)
- [x] GRUB Multiboot2 legacy ISO path (`src/build/lib/kernel.sh`)
- [x] Native UEFI loader (`src/uefi/bootx64.c` → `BOOTX64.EFI`)
- [x] Multiboot2 / UEFI handoff parsing (`src/core/multiboot2.c`)
- [x] Unified build system: shell scripts + Python builder (`src/build/core.sh`, `src/build/popcorn_build/`)
- [x] QEMU UEFI smoke and stability tests (`src/build/lib/qemu-uefi.sh`)
- [x] Boot splash and staged initialization (`src/core/init.c`)

### Console and shell

- [x] VGA text mode abstraction (`src/core/console.c`)
- [x] GOP framebuffer text rendering (UEFI path)
- [x] Scrollback, command history, tab completion (`src/core/kernel.c`)
- [x] Interactive shell with filesystem, editor, and sysinfo commands
- [x] Shared utilities module (`src/core/utils.c`)

### Pop module system

- [x] Module registry and lifecycle (`src/core/pop_module.c`)
- [x] Cursor save/restore pattern documented in `pop.md`
- [x] Shimjapii, Spinner, Uptime, Halt pops
- [x] Filesystem pop — in-memory FS with directories (`src/pops/filesystem_pop.c`)
- [x] Dolphin text editor (`src/pops/dolphin_pop.c`)
- [x] Sysinfo, memory map, and CPU pops (`src/pops/sysinfo_pop.c`, `memory_pop.c`, `cpu_pop.c`)

### Kernel internals (prototype)

- [x] Bitmap physical memory manager + kmalloc (`src/core/memory.c`)
- [x] 4-level VMM: map/unmap, per-task PML4 hooks (`src/core/vmm.c`)
- [x] IDT, PIC remap, PIT timer, PS/2 keyboard IRQs (`src/core/kernel.c`, `src/core/timer.c`)
- [x] Preemptive scheduler skeleton with real `iretq` context switch (`src/core/scheduler.c`, `src/core/context_switch.asm`)
- [x] Syscall table and INT `0x80` dispatch — 21 handlers (`src/core/syscall.c`)
- [x] Serial COM1 debug output (scattered; not yet a formal driver)
- [x] UEFI firmware keyboard fallback (`src/core/uefi_input.c`)

### Bring-up “drivers” (inline, no framework)

These work today as direct port I/O or firmware calls. They will be refactored into the driver framework in Phase 4.

| Device | Location | Notes |
|--------|----------|-------|
| PIC 8259 | `src/core/kernel.c` | Legacy IRQ controller |
| PIT 8254 | `src/core/timer.c` | 100 Hz tick; UEFI path also polls |
| PS/2 keyboard | `src/core/kernel.c` | IRQ1 + scancode map |
| VGA text | `src/core/console.c` | CRTC ports |
| GOP framebuffer | `src/core/console.c`, `src/includes/boot_fb.h` | Blit-only, no modeset driver |
| Serial COM1 | `src/core/scheduler.c`, boot stages | Debug only |
| UEFI ConIn | `src/core/uefi_input.c` | Firmware CR3 switch |

---

## Phase 1 — Stabilize v0.5 (Current focus)

Goal: Reliable boot and shell on QEMU and real hardware before layering new subsystems.

- [ ] Mark all releases as pre-release until Phase 3 exit criteria are met
- [ ] Boot reliably on QEMU (GRUB ISO and UEFI img) — automated `test-uefi` green
- [ ] Boot on real hardware (T440p / Ventoy USB, Secure Boot off)
- [ ] Unify timer delivery: IRQ-driven preemption on UEFI path (replace `kmain` poll-only loop where possible)
- [ ] Replace CPU exception stubs (vectors 0–31 halt forever) with diagnosable handlers — at minimum `#PF`, `#GP`, `#DF`
- [ ] Resolve scheduler bootstrap edge case (`bootstrap_on_kmain_stack` in `src/core/scheduler.c`)
- [ ] Add CI (GitHub Actions): build + QEMU smoke on push
- [ ] Align `readme.md` with actual GUI entry points (`gui-fd.py`, `gui-macos.py`)

**Exit criteria:** Stable shell on both boot paths; exceptions log useful fault info over serial; CI green.

---

## Phase 2 — Kernel foundations for drivers

Goal: Interrupts, memory isolation, and process model stable enough that drivers do not fight the shell or scheduler.

Drivers must not be built on top of the current syscall stubs or shell-direct I/O pattern. Complete Phase 2 first.

### 2.1 Interrupts and time

- [ ] Consistent `sti` / `hlt` idle loop on all boot paths
- [ ] Document and test IRQ nesting policy
- [ ] `request_irq()` precursor: central IRQ dispatch table (replaces ad-hoc handlers in `kernel.c`)
- [ ] HPET or ACPI PM timer probe (optional; keep PIT as fallback)
- [ ] `timer_get_ticks()` and `SYS_GETTIME` backed by a single clock source

### 2.2 Virtual memory

- [ ] Implement 2 MiB PDE splitter for 4 KiB maps in low 1 GiB (`src/includes/vmm.h` gap)
- [ ] Per-process address spaces used by all non-idle tasks
- [ ] User VA region policy: map user pages above 1 GiB with `VMM_PTE_US`
- [ ] Page fault handler: demand-zero, guard pages, copy-on-write hooks (stubs OK initially)
- [ ] DMA-safe buffer allocation helper (physically contiguous, below 4 GiB)

### 2.3 Processes and syscalls (driver-facing subset)

Full POSIX semantics are out of scope for this roadmap, but drivers need a real `ioctl`, file descriptors, and sleep/yield.

- [ ] Dynamic task allocation via kmalloc (replace static 32-task pool)
- [ ] File descriptor table per task (even if only device nodes initially)
- [ ] Rewire `SYS_READ` / `SYS_WRITE` / `SYS_IOCTL` to go through device layer, not console directly
- [ ] `SYS_SLEEP` / `SYS_YIELD` integrated with scheduler wait queues
- [ ] Ring 3 entry: user code segment, `syscall` gate or `int 0x80` with `0xEE` → `0xEF`, TSS IST for user stacks

**Exit criteria:** A kernel thread can block on an IRQ wake; `ioctl` routes to a registered device; page faults on bad user addresses are handled cleanly.

---

## Phase 3 — Driver framework

Goal: A small, explicit device model — not a Linux clone. All Phase 0 inline I/O migrates here.

### 3.1 Directory layout

```
src/drivers/
  core/
    device.c / device.h    — device list, lifecycle, naming (/dev/...)
    driver.c / driver.h    — driver registration, probe, remove
    irq.c / irq.h          — request_irq, enable, disable, shared IRQ
    io.c / io.h            — inb/outb, mmio map, ioremap helpers
    dma.c / dma.h          — bounce buffers, phys addr lookup
  bus/
    pci.c / pci.h          — config space, BAR enumeration, IRQ line
  class/
    chardev.h              — read/write/ioctl ops table
    blockdev.h             — read_blocks/write_blocks, geometry
    fbdev.h                — mmap, mode info, blit ops
```

### 3.2 Core abstractions

- [ ] `struct device` — name, bus, class, driver binding, `void *priv`
- [ ] `struct driver` — name, probe(), remove(), match table
- [ ] `device_register()` / `device_unregister()` — global device list
- [ ] `driver_register()` — attach on probe success
- [ ] Device classes:
  - **char** — byte stream (serial, keyboard)
  - **block** — fixed-size sectors (virtio-blk, ATA)
  - **fb** — framebuffer (GOP handoff → kernel fbdev)
- [ ] `dev_open` / `dev_close` / `dev_read` / `dev_write` / `dev_ioctl` — kernel-side API
- [ ] Syscall path: `SYS_OPEN` opens `/dev/...` nodes backed by `struct device`
- [ ] `ioctl` encoding: class-specific command namespaces in `src/includes/ioctl.h`
- [ ] IRQ layer:
  - `irq_register(irq, handler, dev_id)`
  - `irq_enable` / `irq_disable`
  - Spinlock or interrupt-disable discipline documented for handler vs. bottom-half
- [ ] Init order: `driver_init()` called from `init.c` after IDT/PIC, before pops

### 3.3 Bus support

- [ ] PCI config space read/write (port I/O and MMIO CFG)
- [ ] Walk PCI bus 0, print class/subclass/vendor during boot (debug)
- [ ] Assign IRQ from PCI INT pin → PIC IRQ (IOAPIC deferred)
- [ ] MMIO BAR mapping via VMM (`vmm_map_4k` with `VMM_PTE_NX` for data)

### 3.4 Integration points

- [ ] Refactor `console.c` keyboard input to read from chardev, not raw PS/2 ports
- [ ] Refactor `timer.c` to register as a clock provider
- [ ] Boot info handoff: GOP framebuffer details → `fbdev` from UEFI/Multiboot tags
- [ ] `mon -list` shows loaded drivers and devices
- [ ] Driver developer guide (`drivers.md` or section in `pop.md`)

**Exit criteria:** New hardware support is added by writing a `driver` + `probe` file and registering it — no edits to `kernel.c` shell loop.

---

## Phase 4 — Drivers

Goal: Refactor existing bring-up code into formal drivers, then add storage and QEMU-friendly devices.

### 4.1 Char drivers (refactor first)

| Driver | Source to refactor | Deliverables |
|--------|-------------------|--------------|
| **serial** | COM1 code in scheduler, boot stages | `/dev/ttyS0`, polled and IRQ RX optional |
| **keyboard** | `kernel.c` PS/2 + `uefi_input.c` | Unified `/dev/kbd`, scancode → keycode |
| **null / zero** | New | `/dev/null`, `/dev/zero` for testing read/write paths |

- [ ] Serial driver (`src/drivers/char/serial.c`)
- [ ] Keyboard driver (`src/drivers/char/keyboard.c`)
- [ ] Null and zero drivers (`src/drivers/char/null.c`, `zero.c`)
- [ ] Remove direct PS/2 port access from `kernel.c` once keyboard chardev is default

### 4.2 Timer and console drivers

| Driver | Source to refactor | Deliverables |
|--------|-------------------|--------------|
| **pit** | `src/core/timer.c` | Clock source provider; sysfs-style info command |
| **vga** | `src/core/console.c` VGA path | `/dev/tty0` text mode |
| **fb** | `src/core/console.c` GOP path | `/dev/fb0`, mode info via `ioctl` |

- [ ] PIT clock driver (`src/drivers/char/pit.c` or `src/drivers/core/clock.c`)
- [ ] VGA text driver (`src/drivers/fb/vga.c`)
- [ ] Framebuffer driver (`src/drivers/fb/fbdev.c`) — mmap hook for future user GUIs

### 4.3 Block drivers

| Driver | Priority | Notes |
|--------|----------|-------|
| **virtio-blk** | P0 | Primary QEMU target; PCI virtio 1.0 |
| **ata/ahci** | P1 | Real hardware; optional initially |
| **ramdisk** | P0 | Bootstraps block layer testing without hardware |

- [ ] Block layer (`src/drivers/core/block.c`) — `bio` or simple `block_request` queue
- [ ] Ramdisk driver for unit tests and early bring-up
- [ ] Virtio-blk driver (`src/drivers/block/virtio_blk.c`)
- [ ] ATA/AHCI driver (`src/drivers/block/ahci.c`) — stretch goal within Phase 4
- [ ] `ioctl` for block devices: flush, geometry query
- [ ] Shell command `dev -list` / `dev -info <name>` for driver visibility

### 4.4 Input and platform (stretch within Phase 4)

- [ ] PS/2 controller driver split from keyboard (mouse-ready)
- [ ] ACPI table parse (RSDP → XSDT) for IRQ routing and HPET base — needed before IOAPIC
- [ ] IOAPIC driver for IRQ routing above legacy PIC
- [ ] RTL8139 or e1000 NIC driver — **only after block layer stable**; registers networking as Phase 5

**Exit criteria (driver framework + drivers complete):**

- [ ] All Phase 0 inline I/O refactored into `src/drivers/`
- [ ] Syscalls `open`/`read`/`write`/`ioctl` work on `/dev/ttyS0`, `/dev/kbd`, `/dev/fb0`
- [ ] Block layer reads/writes sectors on virtio-blk in QEMU
- [ ] PCI enumeration lists virtio and any other found devices at boot
- [ ] No driver-specific code in `kernel.c` `execute_command` except dev diagnostics

---

## Phase 5 — Beyond this roadmap (not scheduled here)

These depend on the driver framework but are intentionally out of scope for the roadmap above:

- VFS layer mounting a real on-disk filesystem (ext2, FAT)
- ELF loader and ring-3 userspace
- Persistent FS replacing the in-memory filesystem pop
- Networking stack (virtio-net + minimal IP)
- USB stack (xHCI)
- SMP, per-CPU runqueues

---

## Suggested timeline

Assuming the same pace as v0.5 (~10–15 hours/week):

| Phase | Duration | Target |
|-------|----------|--------|
| Phase 1 — Stabilize | 2–3 months | v0.5.1 pre-release |
| Phase 2 — Kernel foundations | 3–4 months | v0.6 |
| Phase 3 — Driver framework | 2–3 months | v0.7 |
| Phase 4 — Drivers | 3–4 months | v0.8 |

**Driver framework start:** Month 5–6 of this plan (after Phase 2 exit criteria).

**First block driver (virtio-blk):** Month 8–9 (after PCI + block layer).

---

## Version targets

| Version | Milestone |
|---------|-----------|
| v0.5.x | Stabilization, CI, exception handlers |
| v0.6 | IRQ unify, VMM/user VA, syscall/device path |
| v0.7 | Driver framework complete, inline I/O migrated |
| v0.8 | Char + block + fb drivers; virtio-blk in QEMU |

---

## Contributing

We welcome contributions and feedback. When opening a PR against this roadmap:

1. Reference the phase and checkbox item (e.g. "Phase 3.2 — `request_irq`").
2. New drivers must register through `driver_register()` — no new raw port I/O in `src/core/`.
3. Follow the pop module cursor save/restore pattern for any UI-facing pop changes.
4. Update this file when exit criteria for a phase are met.

---
