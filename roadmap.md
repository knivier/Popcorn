# Popcorn — Project Roadmap

## Overview

Popcorn is a modular x86-64 kernel framework for learning operating system development. It ships with a native UEFI loader, GRUB Multiboot2 ISO boot, an interactive in-kernel shell, and a pop-module extension system.

**Coding target met:** v0.7 — Rust driver network, screen backends, Rust pops, kbd/PCI, in-kernel catalog.

**Next version:** v0.8 — block path (ramdisk → virtio-blk), thin remaining C I/O.

**Verified:** 3 Oct 2026 — Phase 1–3 coding gates green in QEMU (`test-uefi`: `#PF` dump, GRUB+UEFI smoke, debugcon `I/B/S/2`). Process leftovers: pre-release marking, real-hardware boot.

**Roadmap scope:** Phase 3 coding largely done. Next is Phase 4 block/storage. VFS, userspace, and a real NIC stack stay in Phase 5.

---

## Maturity Snapshot

| Area | Status | Verified in |
|------|--------|-------------|
| Boot (GRUB + UEFI) | Done | `src/core/kernel.asm`, `src/uefi/bootx64.c`, `scripts/core.sh` |
| Console / shell UX | Done (C client of Rust screen) | `src/core/console.c`, `src/core/shell.c` |
| PMM + kmalloc | Done (prototype) | `src/core/memory.c` |
| VMM (4-level paging) | Partial | direct map 64 GiB; process PML4 clones upper 256; no PDE splitter yet |
| PMM (bitmap) | Done (≤16 GiB track) | `memory.c` / `physmap.h` — was hard-capped at 1 GiB |
| Scheduler + context switch | Partial | `src/core/scheduler.c` — wait queues, sleep, bootstrap guard |
| Syscalls (14 registered) | Partial | `src/core/syscall.c` — real fd/dev/time/heap/cwd only |
| FAT32 on selected disk | Done (no VFS) | `rust/popcorn_kernel/src/fs/` (the in-memory FS pop was removed) |
| Pop modules | 6 Rust + FS/Dolphin C | `pops/*`, `src/core/pop_module.c` |
| Device / drive table | Done | Rust registry + C fd bridge |
| Registry catalog (RAM DB) | Done | `catalog.rs` / `catalog.h` — `catalog` shell cmd |
| IRQ table | Done (C + Rust claims) | `irq.c`, `drivers/irq.rs` |
| Rust crate | Active | abi/device/driver/class/dma/pit + backends |
| PCI walk | Done (bus 0) | `drivers/bus/pci.rs` |
| Block / storage | Done (ram / virtio / NVMe / USB MSC) | `drivers/block/`, `drivers/usb/` |
| CI | Done | `.github/workflows/ci.yml` — build + `test-uefi` |

---

## Language split (v0.7 policy)

New kernel work lands in Rust unless it has to stay C/asm.

| Stay C / asm | Move to Rust in v0.7 | Later (after v0.7) |
|--------------|----------------------|--------------------|
| `kernel.asm` long-mode entry | Driver network (`device` / `driver` / `irq` / `io`) | VMM helpers |
| `context_switch.asm`, IDT/TSS setup | VGA text + GOP framebuffer backends | Wait-queue rewrite |
| `kmain` input loop (until kbd chardev) | Serial, null/zero, PIT clock provider | ELF / ring 3 |
| `memory.c` PMM (allocator shim only) | Pop registry + simple pops | Dolphin |
| Linker script `link.ld` | `console_*` writing via `/dev/tty0` + `/dev/fb0` | virtio-net |

C calls a small `extern "C"` surface (`driver_init`, `irq_dispatch`, `dev_read` / `dev_write`, `pop_register` / `pop_run`). Rust owns registration, probe, and class ops. Panic in Rust = serial dump + halt, same as a CPU exception.

Do **not** rewrite `console.c` UX (scrollback, history, status bar) in the first Rust PR. Split the **backends** (0xB8000 CRTC + GOP blit) into drivers; keep the 80×25 cell API as a client of those drivers.

---

## Phase 0 — Completed (v0.5)

Verified present in tree.

### Boot and build

- [x] x86-64 long-mode entry with identity map and high-half kernel (`src/core/kernel.asm`, `src/link.ld`)
- [x] GRUB Multiboot2 legacy ISO path (`scripts/lib/kernel.sh`)
- [x] Native UEFI loader (`src/uefi/bootx64.c` → `target/BOOTX64.EFI`)
- [x] Multiboot2 / UEFI handoff parsing (`src/core/multiboot2.c`)
- [x] Unified build system: shell scripts + Python builder (`scripts/core.sh`, `scripts/popcorn_build/`)
- [x] QEMU UEFI smoke and stability tests (`scripts/lib/qemu-uefi.sh`)
- [x] All artifacts under `target/`; platform GUIs: macOS / Fedora / Linux / Windows(WSL)
- [x] Boot splash and staged initialization (`src/core/init.c`)
- [x] GOP framebuffer text on no-CSM machines (`src/core/console.c` `console_fb_*`, `src/includes/boot_fb.h`)
- [x] UEFI firmware keyboard fallback (`src/core/uefi_input.c`)

### Console, pops, internals

- [x] VGA text + GOP shadow buffer, scrollback, history, tab completion
- [x] Pop registry (`src/core/pop_module.c`, max 10) — Shimjapii, Spinner, Uptime, Filesystem, Sysinfo, Memory, CPU, Dolphin
- [x] Bitmap PMM + kmalloc; `vmm_init` from `memory_init`; 4-level map/unmap; `vmm_map_kernel_region`
- [x] IDT, PIC remap, PIT, PS/2 IRQ1, INT `0x80` (DPL 3 gate `0xEE`)
- [x] Preemptive scheduler skeleton with real `iretq` (`src/core/context_switch.asm`)
- [x] 21 syscall handlers registered (behavior is still stubby — see Phase 1 / 2)

### Bring-up I/O (inline, no framework)

These work as direct port I/O or firmware calls. v0.7 moves them into the Rust driver network.

| Device | Location today | v0.7 destination |
|--------|----------------|------------------|
| PIC 8259 | `src/core/kernel.c` `idt_init` | Rust `irq` + PIC backend |
| PIT 8254 | `src/core/timer.c` | Rust clock provider (`/dev` clock, 100 Hz) |
| PS/2 keyboard | `src/core/kernel.c` | Rust chardev `/dev/kbd` (after IRQ table) |
| VGA text | `src/core/console.c` → `0xB8000` + CRTC `0x3D4/0x3D5` | Rust `/dev/tty0` |
| GOP framebuffer | `src/core/console.c` `console_fb_*` | Rust `/dev/fb0` |
| Serial COM1 | `src/core/scheduler.c` `serial_putc`, boot stages | Rust `/dev/ttyS0` |
| UEFI ConIn | `src/core/uefi_input.c` | Stay C until kbd chardev exists |

---

## Phase 1 — v0.5 leftovers

Coding gate items are done (QEMU + CI). Remaining are release/process and hardware soak.

- [ ] Mark releases pre-release until v0.7 exit criteria
- [x] Keep `test-uefi` green on GRUB ISO and UEFI img
- [ ] Boot on real hardware (T440p / Ventoy USB, Secure Boot off)
- [x] Diagnosable CPU exceptions — `#PF`, `#GP`, `#DF` dump CR2 / error / RIP on COM1, then halt
- [x] Stop forcing poll-only time in `kmain` (UEFI poll; GRUB IRQ PIT)
- [x] One task/stack allocator — shared `g_task_pool[32]`
- [x] Scheduler bootstrap: `scheduler_end_bootstrap()` clears tick skip
- [x] CI: GitHub Actions running `./scripts/core.sh test-uefi` (builds + UEFI/GRUB/#PF smoke)

**Exit criteria:** `#PF` prints a useful serial dump in QEMU; GRUB path can use IRQ time; CI green.

---

## Phase 2 — Driver-facing kernel (overlap with v0.7)

Do **not** build the driver network until this subset is in place. Finish below; full POSIX and ring 3 can wait.

### Must land with v0.7

- [x] Central IRQ table: `irq_register` / `irq_enable` / `irq_disable` (replaces hard-coded `idt_set_gate(0x20/0x21)` in `kernel.c`)
- [x] Idle `sti; hlt` on paths that are not stuck in UEFI poll mode
- [x] `timer_get_ticks()` + `SYS_GETTIME` from one clock source
- [x] Per-task fd table; `SYS_OPEN` / `READ` / `WRITE` / `IOCTL` go through `dev_*`, not the console
- [x] `SYS_SLEEP` / `SYS_YIELD` on scheduler wait queues (IRQ wake)

### Memory architecture (do before heavy v0.8 block / userspace)

Why DRAM looked “stuck at 1 GiB”: QEMU `-m 1024` **and** a PMM bitmap hard-capped at the first gigabyte. Boot already identity-mapped 64 GiB; the allocator simply refused to track the rest.

- [x] Shared `physmap.h` — `VMM_DIRECT_MAP_BASE`, 64 GiB identity window, 16 GiB PMM track cap
- [x] PMM bitmap tracks up to 16 GiB from the multiboot/UEFI mmap (no 1 GiB fallback clamp)
- [x] Direct-map accessors (`phys_to_virt` / table walks via high-half)
- [x] Process PML4 split: clear lower 256, copy upper 256 from master (`vmm_clone_kernel_space`)
- [x] `vmm_init_process_address_space` uses clone (not a fresh 1 GiB-only layout)
- [x] QEMU UEFI smoke default RAM raised to 4 GiB (`-m 4096`)
- [x] Non-idle tasks get a private PML4; idle keeps master; `vmm_load_cr3` on switch
- [x] 2 MiB PDE splitter — `vmm_map_4k` can overlay identity/direct-map windows
- [ ] User VA mappings with `VMM_PTE_US` in PML4[1..] (ring 3 prep; identity still shared at PML4[0])

### Can slip to v0.8+

- [ ] `#PF` policy: demand-zero / guard / COW hooks
- [ ] DMA helper (contiguous, below 4 GiB) — stub exists in Rust `dma.rs`
- [ ] Dynamic tasks via kmalloc (delete both static pools)
- [ ] Ring 3: user CS, `0xEE` → `0xEF` or `syscall`, TSS IST for user stacks
- [ ] HPET / ACPI PM timer (optional; PIT stays fallback)

**Exit criteria for the v0.7 subset:** a kernel thread can block on an IRQ wake; `ioctl` reaches a registered device.

**Verified in QEMU:** boot `phase2_selftest()` emits debugcon `I` (`SYS_IOCTL`/`OPEN`/`WRITE` → fd → console/null), `B` (wait-queue park/`wake_all`), `S` (sleep-deadline via `wake_expired_sleepers`), `2` (all passed). UEFI + GRUB smoke assert these tags.

---

## Phase 3 — v0.7: Rust crate, driver network, screen, pops

**Current focus.** `kernel.c` is thin (`kmain`); IDT/kbd/shell live in their own C files. Rust drive network starts `null`/`zero`/`ttyS0`/`tty0`/`fb0` at boot (`init_drive`); shell `drive`/`init_drive`/`dev` commands. Display (VGA CRTC + GOP font blit) is the Rust screen drive — `console.c` only calls `rust_screen_*`.

### 3.1 Rust build (do first)

Crate: `src/rust/popcorn_kernel` (workspace) producing `libpopcorn_kernel.a`, linked by `scripts/lib/kernel.sh` and `scripts/popcorn_build/builder.py`.

```
src/rust/
  Cargo.toml                         — workspace, panic=abort, no_std
  popcorn_kernel/
    src/lib.rs                       — rust_init() extern "C"
    src/abi.rs                       — C structs (PopModule, CatalogEntry, ioctl)
    src/catalog.rs                   — in-kernel registry DB (drives/devs/pops/irqs/syscalls)
    src/drivers/                     — driver network
      mod.rs
      device.rs / driver.rs / irq.rs / io.rs / dma.rs
      bus/pci.rs
      class/{chardev,blockdev,fbdev}.rs
      backends/{serial,vga,fb,pit,null,zero,kbd,screen,mem,cpu,clock}.rs
      registry.rs                    — init_drive + /dev publish
    src/pops/                        — Rust pops (no halt toy)
  .cargo/config.toml                 — x86_64-unknown-none
```

- [x] `#![no_std]`, `x86_64-unknown-none` (or custom), `panic=abort` (halt loop for now; COM1 dump later)
- [x] `cargo build` produces a staticlib; `ld` pulls it with existing `link.ld`
- [x] `GlobalAlloc` shim → C `kmalloc`/`kfree` (`alloc_shim.rs`); `Box` probe emits debugcon `a`
- [x] Link fix: nightly `build-std` (`core`/`alloc`/`compiler_builtins`) with `-C code-model=large`; link full `libpopcorn_kernel.a` (prebuilt sysroot uses `R_X86_64_32`, unusable in high-half)
- [x] `rust_init()` called from `init.c` after IDT/PIC, before C pop registration — prints `Rust active`
- [x] rustc added to `core.sh` dependency check (and documented for Fedora / macOS / Debian)

### 3.2 Driver network (Rust)

A small, explicit device model — not a Linux clone. “Driver network” here means the registry + class graph + IRQ routing, **not** an Ethernet stack (that is Phase 5).

- [x] Drive catalog + `init_drive` / probe (`drivers/registry.rs`); boot starts builtins from `rust_init` / `init_drives`
- [x] `/dev` nodes via `device_register_rust` → C fd/`SYS_*` bridge
- [x] Char backends: `null`, `zero`, `ttyS0`; screen: `tty0` (VGA), `fb0` (GOP present ioctl)
- [x] Shell: `drv list|load|info|cmd`, `dev list`
- [x] Info drives: `mem`, `cpu`, `clock` (PIT uptime); block I/O deferred (FS stays RAM)
- [x] `ioctl` namespaces in `src/includes/ioctl.h` (mem/cpu/clock)
- [x] Classes sketched: `class/{chardev,blockdev,fbdev}.rs` (+ `device`/`driver`/`dma`/`irq`/`pit`)
- [x] Rust IRQ claim table (`drivers/irq.rs`); PIC enable/EOI still C — IOAPIC later
- [x] In-kernel catalog DB (`catalog.rs`) — shell `catalog` lists drives/devs/pops/syscalls

**Exit criteria:** adding a device is a Rust `Driver` + `probe` — no edits to the `kernel.c` shell loop.

### 3.3 Screen writing → drivers (Rust)

Today every `console_putchar` eventually writes either VGA memory at `0xB8000` (plus CRTC cursor ports) or a RAM shadow that `console_fb_sync_*` blits to the GOP linear framebuffer. That backend split moves into the driver network. The 80×25 cell API stays so pops and the shell do not change in the first cut.

| Today | v0.7 driver | Ops |
|-------|-------------|-----|
| `vga_memory` + `update_hardware_cursor` | `/dev/tty0` VGA text | cell write, cursor, palette |
| `console_fb_init` / `console_fb_sync_cell` | `/dev/fb0` GOP handoff | mode info, blit, present |
| `console_putchar` / `console_print*` | console core (C or thin Rust) | talks to tty0 **or** fb0, never ports |

- [x] VGA text driver (`backends/vga.rs`) — CRTC cursor + cell write; console uses `rust_screen_set_cursor`
- [x] Framebuffer driver (`backends/fb.rs`) — GOP font blit + dirty present owned in Rust
- [x] Full cell blit path owned by display drive (`screen.rs` / `fb.rs`); console UX calls `rust_screen_*`
- [x] Shell, status bar, scrollback still work on GRUB (VGA) and UEFI (fb) — smoke green
- [x] FS/Dolphin use `console_*` (no legacy `vidptr` / raw cell poke)

**Exit criteria:** `console.c` contains no `write_port(0x3D4/0x3D5)` and no GOP pixel loops; those live in Rust drivers.

### 3.4 Pops → Rust

C registry is a 10-slot array and `void (*pop_function)(unsigned int)`. Rust keeps that ABI so `init.c` can register either side during the transition.

**Split (current intent):** pops are UX/aggregation; durable I/O and hardware state live in drivers.
- Filesystem is now Rust FAT32 (`fs/`) over the **block drivers** (ramdisk / virtio / NVMe / USB MSC); the old in-memory FS pop is gone.
- Memory / CPU are **drivers** (or drive-backed info); pops (or shell) call them.
- Sysinfo is a **pop** that aggregates multiple drive/info commands.
- Halt pop and shell `hang`/`halt` toys are removed.

| Order | Pop | Why this order |
|-------|-----|----------------|
| 1 | Shimjapii, Spinner, Uptime | Tiny; prove ABI + cursor save/restore |
| 2 | Sysinfo, CPU, Memory | Thin UX over memory/cpu/info drives |
| 3 | Dolphin (FS is now Rust FAT32, not a pop) | Large, stateful; sits on the FAT32 shell API |

- [x] Rust `pops::registry` + `rust_pops_register` with the same `PopModule` layout
- [x] Port Shimjapii, Spinner, Uptime; drop the C files from `scripts/lib/kernel.sh`
- [x] Remove Halt pop + shell `hang` / `halt` commands
- [x] Port Sysinfo / CPU / Memory as pops over `mem`/`cpu`/`clock` drives
- [x] In-memory filesystem pop replaced by FAT32 on the selected block disk; Dolphin saves through it
- [x] Update `pop.md` for Rust pops (cursor save/restore still required)

### 3.5 First char / clock drivers (same crate)

- [x] Serial `/dev/ttyS0` — Rust chardev (early `boot_serial_putc` still used pre-driver)
- [x] Null / zero — Rust chardevs via fd bridge
- [x] PIT clock info drive (`clock` / `/dev/clock`) — `timer.c` still owns IRQ/poll
- [x] PCI config walk (bus 0 print at boot) — `drivers/bus/pci.rs`
- [x] Delete leftover `serial_putc` from `scheduler.c` — uses shared `boot_serial_putc`

**v0.7 exit criteria:**

- [x] `cargo` + existing C toolchain produce one kernel; `test-uefi` green
- [x] Screen pixels/CRTC owned by Rust `tty0`/`fb0` backends (`rust_screen_*`; console is a client — not every putchar via `open`/`write` fd)
- [x] At least three pops are Rust and registered through the Rust registry (six: shimjapii/spinner/uptime/memory/cpu/sysinfo)
- [x] Serial, null/zero, kbd are Rust devices (`dev list` / `drive list`). `mon -list` lists tasks only
- [x] No new device port I/O under `src/core/` for drives that exist in Rust (kbd trampoline + PIC EOI only). `timer.c` / early `boot_serial_putc` remain until clock IRQ moves

---

## Phase 4 — v0.8: remaining drivers

Refactor the rest of Phase 0 I/O, then storage.

### 4.1 Input

- [x] Keyboard chardev `/dev/kbd` (PS/2; C trampoline for IRQ/poll/EOI)
- [x] Shell/`kmain` read scancodes via `/dev/kbd` device ops (`kbd_read_scancode`)
- [x] PS/2 controller split (keyboard vs mouse/aux channels; aux disabled until mouse drive)

### 4.2 Block (Windows-safe)

**Policy (picker):** boot/USB medium auto-selected and writable. `internal`
(NVMe/SATA / QEMU second virtio) stays **LOCKED** until `install <name> YES`.
ThinkPad Windows NVMe is never written unless the user explicitly unlocks it.
QEMU: virtio[0]=`usb0` (`popcorn-data.img`), virtio[1]=`nvme0` (`popcorn-internal.img`).

| Wave | Deliverable | Status |
|------|-------------|--------|
| 1 | Disk registry + `disk list/use/info/read/write`; `ram0`; QEMU virtio `usb0` | Done |
| 1b | Multi-virtio + picker; `install … YES`; QEMU locked `nvme0` stand-in | Done |
| 2 | Real NVMe driver + `install … YES`; QEMU `-device nvme` | Done |
| 3 | Real USB MSC over xHCI (`usb0`); virtio fallback | Done |
| 4 | AHCI/SATA (if hardware lacks NVMe) | Later |

| Driver | Priority | Notes |
|--------|----------|-------|
| **ramdisk** (`ram0`) | P0 | Always present; proves write gate |
| **virtio-blk** (`usb0`/`nvme0`) | P0 | QEMU stand-ins; legacy `disable-modern=on` |
| **usb** (`usb0` real) | P1 | ThinkPad persistent store |
| **ata/ahci/nvme** | P2 | Internal; locked until install |

- [x] Write-gated disk registry (`drivers/block/`, `disk.h`)
- [x] Ramdisk `ram0` (4 MiB, 512 B sectors)
- [x] Multi virtio-blk + boot auto-select / internal lock
- [x] Shell `disk` + `install <name> [YES]`
- [x] QEMU two-disk picker test images
- [x] Wave 2: NVMe driver (AHCI later)
- [x] Wave 3: USB MSC over xHCI
- [x] FAT32 on selected disk (format-on-first-use; shell ls/write/read/…)
- [ ] Full VFS / multi-mount (later)

### 4.3 Platform (stretch)

- [ ] ACPI (RSDP → XSDT) for IRQ routing / HPET — before IOAPIC
- [ ] IOAPIC
- [ ] PDE splitter + MMIO BAR map (`vmm_map_4k` + `VMM_PTE_NX`) if still open from Phase 2
- [ ] NIC (RTL8139 / e1000 / virtio-net) **only after** block is stable — networking is Phase 5

**Exit criteria:** Phase 0 inline I/O is gone from `src/core/`; `open`/`read`/`write`/`ioctl` work on `ttyS0`, `kbd`, `fb0`; QEMU reads virtio-blk sectors; PCI lists devices at boot.

---

## Phase 5 — Beyond this roadmap

Depends on the driver network; not scheduled:

- VFS layer + a second on-disk FS (ext2); FAT32 already exists but is called directly by the shell
- ELF loader and ring-3 userspace
- Networking stack (virtio-net + minimal IP)
- USB hubs / HID (xHCI mass storage already works)
- SMP, per-CPU runqueues

---

## Suggested sequence (preparing for v0.7)

| Step | What | Language | Done when |
|------|------|----------|-----------|
| 1 | Exception dump + CI + readme + single task pool | C + Actions | QEMU `#PF` prints CR2; `test-uefi` in CI |
| 2 | Rust crate skeleton + `rust_init` | Rust | `libpopcorn_kernel.a` links; boot prints `Rust active` |
| 3 | Driver network + IRQ table | Rust (+ C IDT trampoline) | PIT/keyboard register instead of hard-coded gates |
| 4 | `/dev/tty0` + `/dev/fb0`; console is a client | Rust | No CRTC/GOP loops in `console.c` |
| 5 | Rust pops: shimjapii, spinner, uptime | Rust | C files removed from the link line |
| 6 | Serial + null/zero; `mon -list` | Rust | Boot logs through `/dev/ttyS0` |
| 7 | (v0.8) kbd, ramdisk, PCI, virtio-blk | Rust | Sectors in QEMU |

Assuming ~10–15 hours/week:

| Phase | Duration | Version |
|-------|----------|---------|
| Phase 1 leftovers + crate link | 3–6 weeks | v0.6 (crate + dumps) |
| Driver network + screen + first Rust pops | 2–3 months | **v0.7** |
| Remaining drivers + virtio-blk | 3–4 months | v0.8 |

---

## Version targets

| Version | Milestone |
|---------|-----------|
| v0.5 | Shipped: dual boot, shell, pops, prototype MM/sched/syscalls |
| v0.6 | Exception dumps, CI, Rust crate links, IRQ table |
| **v0.7** | Driver network in Rust; screen via `/dev/tty0` + `/dev/fb0`; first Rust pops; serial/null devices |
| v0.8 | Keyboard + block + PCI; virtio-blk in QEMU; remaining C I/O gone from `core/` |

---

## Contributing

When opening a PR against this roadmap:

1. Reference the phase and checkbox (e.g. "Phase 3.3 — `/dev/fb0`").
2. New drivers register through the Rust `driver_register` path — no new raw port I/O in `src/core/`.
3. New pops are Rust unless they are Filesystem/Dolphin still waiting on the console split. Cursor save/restore still applies (`pop.md`).
4. Do not rewrite boot, context switch, or the shell loop “because Rust” — those stay C/asm until a later version names them.
5. Update this file when a phase’s exit criteria are met.
