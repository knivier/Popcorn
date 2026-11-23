# Design review: universal driver framework + virtio-net

Status: **M0 and M1 done (23 Nov 2026); M2 next.** See "Progress" below for what landed and what changed against this plan. Originally: This review merges three competing drafts (performance-first, grants-first, ship-first). The grants-first structure won, and the best ideas from the other two were grafted in. Every error the reviewers flagged has been re-checked against the repo and the OASIS VIRTIO 1.2/1.3 text and fixed.

## Summary

Popcorn has no network code. The kernel delivers only IRQ0 and IRQ1, through the 8259. Every storage driver polls, and each one reaches hardware through raw `inl`/`outl` and `phys_to_virt`. This proposal adds four things:

1. A small driver framework: hardware grants, one queue trait, LAPIC + MSI-X with a poll fallback, and IRQ-safe locks.
2. One shared virtio layer, with a split virtqueue plus modern and legacy PCI transports.
3. A modern (VERSION_1) virtio-net driver.
4. A hand-written no_std stack (Ethernet, ARP, IPv4, ICMP, UDP, DHCP, DNS) that sees only a `NetDevice` trait.

The work lands in five milestones, and each one ends with a check you can see in the serial log or a pcap:

| M | Lands | Visible result |
|---|---|---|
| M0 | Framework, plus the virtqueue pulled out of `virtio_blk.rs` | (see §8) |
| M1 | Polled virtio-net | `ping 10.0.2.2` |
| M2 | LAPIC + MSI-X hybrid completions | (see §8) |
| M3 | UDP / DHCP / DNS | (see §8) |
| M4 | virtio-blk moved onto the framework | Last raw port I/O removed from virtio drivers |

The review also found two problems with the starting assumptions:
- **QEMU very likely already adds a NIC today.** No QEMU invocation passes `-nodefaults` or any `-nic`/`-net`/`-netdev`, so QEMU adds its default NIC: e1000e on q35, e1000 on pc.
- **The UEFI "sti hangs" problem has a concrete suspect.** Every IDT vector outside 0–31, 0x20, 0x21 and 0x80 is not-present.

### Progress

**Already landed before M0 (interrupt work, 23 Nov 2026).** These supersede parts of §1 (F4, F5, F11) and §3.3.1:
- Every IDT vector (0–255) has a stub that builds one `PopFrame` (`src/includes/popframe.h`) with FXSAVE below it and calls `isr_dispatch` (`src/core/isr.c`); there is no separate `irq_vectors.asm`/`vector_common`.
- UEFI is no longer poll-only: `src/core/lapic.c` puts the LAPIC in virtual-wire mode (LINT0 ExtINT, LINT1 NMI, LAPIC timer + error LVT masked, spurious 0xFF) and masks every IOAPIC entry; IRQ0 drives preemption on both boot paths, with PIT polling only as a fallback. Q1 is settled.
- Tasks switch by interrupt frame (`int 0x81` for voluntary switches). The LAPIC stays in C (Q5): Rust reaches it through `lapic_eoi` / `lapic_self_ipi`.

**M0 (23 Nov 2026), owner answers applied:** Q14 descriptive names; Q13 deprecation warnings + lint; Q8 a separate `map_mmio_uc` (no page splitting); Q9 cleanup **inside** M0.
- `sync.rs`: `IrqSpinLock` (interrupts off; re-entry panics with both call sites), `TaskLock` (yielding mutex; reclaims a killed holder), `Once`. `util/copy.rs` (`rep movsb`/`stosb`), `util/csum.rs` (RFC 1071).
- Panic handler prints `*** RUST PANIC: <msg> at file:line ***` on COM1 only (`com1_putc`/`com1_puts`), then halts.
- **All 69 `static mut` in the Rust crate converted** (display, storage, fs/editor, registries) to atomics, `IrqSpinLock`, `TaskLock` or `Once`; lock order is documented at each static (editor → fs → block table; block table is a leaf; screen → fb). Slab allocator in `alloc_shim.rs` (classes 16–2048 B, counters).
- Grants: `drivers/grant/{mmio,port,dma,irq}.rs` — `MmioRegion` (UC, bounds-checked, never panics, `GRANT_VIOLATIONS`), `PortRange`, `DmaPool` (64 KiB runs, page-safe `BufSlab`), vector table + allocator (`rust_vector_dispatch` from `isr_dispatch`). Kernel self-test vector is **0x30** (not 0x40). `drivers::io` is `#[deprecated]`; the build warnings list the drivers still to migrate.
- `drivers/queue.rs` (`Queue`, `Seg`, `Completion`, `DeviceAddr`), `bus/pci.rs` (CF8/CFC lock with atomic RMW, capability walk, I/O BAR sizing, `PciFunction`, `claim`/`release`, `msix_info`, `pci -caps`).
- `drivers/virtio/`: `transport.rs`, `modern.rs`, `legacy.rs` (dynamic 20/24 config offset), `pci.rs` (selection), `virtqueue.rs` (`SplitQueue`, pure `core`), `probe.rs`.
- Shell: `irq`, `pci -caps`, `virtio -probe`. Scripts: `core.sh lint` (`scripts/lib/lint.sh`), `core.sh test-host` (`src/rust_host_tests`).

**M0 exit, verified in QEMU on UEFI and GRUB** (`driver_framework_m0_irq_pci_virtio` in `test-shell`):
- the LAPIC self-IPI on vector 0x30 is counted exactly once at boot (`irq: selftest vector 0x30 count=1`);
- `pci -caps` shows MSI-X on virtio-blk, NVMe and xHCI, and the virtio vendor caps (common/notify/device/isr/pci-cfg) on the modern device;
- `virtio -probe` drives a modern-only virtio-blk (0x1042, ignored by the legacy driver) through claim → ModernTransport → negotiate → `SplitQueue` on a `DmaPool`, reads its capacity (16384 sectors) and sector 0 (the harness's `POPCORN-VIRTIO-M0` signature), resets and releases it — twice in a row. (This replaces "compare capacity with the legacy driver": resetting a device the legacy driver is using would break it.)
- Host tests green (11): checksum vs a byte-wise reference at every length/alignment, RFC 1071 vectors; SplitQueue against a random-order device model past the u16 wrap (>70k completions) with and without EVENT_IDX, arm race, full ring, kick suppression, legacy layout.

**M1 (23 Nov 2026):** polled virtio-net + Ethernet/ARP/IPv4/ICMP.
- `drivers/pci_drivers.rs`: the PCI driver registry (name, class, ID table, probe); the matcher claims each function for one driver. virtio-net is its first entry (modern 0x1041, and transitional 0x1000 with subsystem 1).
- `drivers/net/mod.rs`: the `NetDevice` trait (zero-copy `RxBatch`, lent `TxBuf`, one `tx_flush` doorbell) + `loopback.rs`. Simplified from §6: no `caps`/`arm` yet (M2/M3).
- `drivers/virtio/net.rs`: modern-only, 256-entry RX/TX `SplitQueue`s, RX at slot+38 (IPv4 at 64), TX header at +52; reset before bus mastering; no notify before DRIVER_OK; `shutdown` resets before the pool is freed.
- `net/{eth,arp,ipv4,icmp,iface}.rs`: pure `core`, no heap; 16-entry ARP cache (60 s), 8 frames pending ARP (3 requests 1 s apart), 16-entry echo table, per-reason drop counters with `rx_frames == delivered + dropped`.
- `net_cmd.rs`: kmain-loop pump (`rust_net_pump`, try-lock only), `net -info/-stats/-up/-down/-arp`, `ping <ip> [count] [size]`.
- QEMU: `qemu_net_args` (user netdev, `disable-legacy=on,romfile=,vectors=0`, `POPCORN_QEMU_NET=off` for no NIC, `POPCORN_NET_PCAP` for a capture), `bootindex=0` on the USB boot stick.
- **Exit, verified on UEFI and GRUB** (`net_m1_virtio_ping_gateway`): `net: probe ok mac 525400123456`, `net: arp 10.0.2.2 52550a000202`, `net: ping 4/4`, 3/3 with 1000-byte payloads, all 256 RX buffers reposted, an unanswered address times out. Also observed: `ping 8.8.8.8` → 3/3 (30–40 ms) through slirp from this host, so only DNS (M3) stands between this and `ping google.com`.
- Host tests: 23 (12 new: ARP byte-exact, 20 pings across a simulated two-machine wire, off-link via gateway, ARP timeout timing, malformed IPv4 table, 200k random/mutated frames with no panic and no buffer leak, loopback).
- Not done yet from §8 M1: the separate `test-net` verb, pcap checker and `net-selftest` build (the `test-shell` case covers the exit instead); `net -down` unconfigures the stack but keeps the device bound.

### Locked decisions (not reopened here)

1. **Grants.** Drivers stay in ring 0 (Rust). They touch hardware only through handles granted at probe: `MmioRegion`, `PortRange`, `IrqHandle`, `DmaPool`. Driver code never calls raw `inl`/`outl` or `phys_to_virt`, so a driver can later move to a ring-3 realm by swapping the grant implementation.
2. **One queue shape** for every device class (block, net, usb, display): submit, then kick (one batched doorbell), then reap completions.
3. **Hybrid completions.** A LAPIC + MSI/MSI-X interrupt wakes the driver. The driver polls up to a budget, then re-arms. Polling remains the fallback when there are no IRQs (UEFI today). PCI devices never use legacy PIC INTx.
4. **Hand-written no_std stack first** (Ethernet/ARP/IPv4/ICMP/UDP/DHCP). TCP vs smoltcp is decided later. virtio-net on QEMU user-mode netdev (10.0.2.x) comes first.

---

## 1. Ground truth from the tree (verified)

| # | Fact | Where | Consequence |
|---|---|---|---|
| F1 | `kmalloc` rounds every allocation up to whole 4 KiB pages, and **every** allocation (any size) is a first-fit `pmm_alloc_contig` bitmap scan via `zone_alloc`. There are only 2048 `mem_block` slots; `zone_alloc` scans them linearly for a free slot and `kfree` (`find_block`) scans them linearly too. `MEM_ALLOC_DMA` limits the scan to frames below 4 GiB. Rust `GlobalAlloc` calls `kmalloc` directly. | `src/core/memory.c` (`MAX_MEMORY_BLOCKS 2048`, `zone_alloc`, `pmm_alloc_contig`, `find_block`), `alloc_shim.rs` | **The hot path must allocate nothing.** Huge contiguous arenas are also risky, so DMA pools are allocated in 64 KiB chunks (§3.1.5). |
| F2 | `virtio_blk.rs` is legacy PIO only. `REG_CONFIG = 20` is hard-coded (line 39). Each device has one preallocated 4 KiB DMA request page (header/data/status) reused for every sector, always on descriptors 0..2, so queue depth is 1. It spins up to 10M times with `fence(SeqCst)` on every iteration; on timeout it marks the disk `dead` and refuses all further I/O (the request page is never freed, since the device may still DMA into it). | `drivers/block/virtio_blk.rs` | Legacy virtio is proven on this kernel, but the code has the anti-patterns we must not copy (depth 1, fence-per-spin, whole-device death on one timeout). It also has a latent bug: device config moves to offset 24 once MSI-X is enabled. |
| F3 | Below 64 GiB, `map_mmio` returns `phys_to_virt`, which is the **write-back** direct map. Only ranges above 64 GiB get UC 4K pages (PWT\|PCD) in the PML4[256] window. | `drivers/dma.rs:56` (`map_mmio`; direct-map return at :61-62, UC flags at :45, window at :49) | The LAPIC (0xFEE00000), MSI-X tables and 32-bit BARs are UC today only because firmware MTRRs make them UC. Grants must not depend on that (§3.1.1, Q8). |
| F4 | `idt_init` installs per-vector exception stubs for 0–31 (#DF on IST2, #GP/#PF on IST1) and 0x80 (DPL3, `0xEE`). `irq_init` installs present gates for **all** PIC vectors 0x20–0x2F (`irq_stub_table`) before anything is registered. **Vectors 0x30–0x7F and 0x81–0xFF are still not-present.** | `src/core/idt.c:124–146`, `irq.c:29–46`, `idt.asm` (`exc_stub_table`, `irq_stub_table`) | A stray or latched vector ≥ 0x30 (LAPIC, MSI-X) raises #NP, which escalates to #GP or #DF. PIC vectors, including spurious IRQ7/15, are covered now, so missing PIC gates no longer explain the comment "UEFI/QEMU: sti+hlt hangs on latent IRQ" (`kernel.c:182`). Fill 0x30–0xFF with catch-alls before enabling the LAPIC or any MSI-X vector (§3.3.1). |
| F5 | kmain: the PIT mode is chosen in `init_transition_to_console()` (`init.c:343`). On UEFI it is poll mode: IF stays 0 and the idle loop runs `timer_poll()` + `pause`. On GRUB it is IRQ0 mode: kmain does `sti` once and idles with `sti; hlt`. `timer_poll()` is still called on every iteration but returns immediately. | `src/core/kernel.c:49–187` (`sti` gate :79, `timer_poll` :97, idle :182–187), `timer.c:106–121` | Poll mode is the **default** on UEFI, not a corner case, so the framework must perform well with no interrupts at all. Shell commands block the loop, so a blocking `ping` must pump `timer_poll()` itself. |
| F6 | No QEMU command line has `-nodefaults`, `-nic none` or `-netdev`. | `scripts/lib/qemu-uefi.sh` (`qemu_uefi_usb_args`, also used by `qemu_uefi_shell_boot`; `qemu_uefi_run_interactive`; `qemu_legacy_shell_boot`), `scripts/lib/kernel.sh` (`run_legacy_qemu`, `qemu_legacy_smoke`) | QEMU adds a default NIC (q35: e1000e `8086:10D3`; pc: e1000 `8086:100E`). Adding an explicit `-netdev` removes it. Confirm with today's boot PCI print. |
| F7 | `#[panic_handler]` spins silently. | `src/rust/popcorn_kernel/src/lib.rs:21` | A panic during NIC bring-up looks like a hang. Print `PanicInfo` first (M0). |
| F8 | `boot_serial_putc` (`multiboot2.c:346`) writes **both** debugcon 0xE9 and COM1. The boot tests match single characters anywhere in the debugcon log: `'M'` (`qemu-uefi.sh` `qemu_uefi_test_stability` `grep -q 'M'`, the `*M*` / `*M*I*B*S*2*` cases in `qemu_uefi_test_alive` / `qemu_uefi_test_debugcon`, `kernel.sh:349–360`). The install selftest is windowed: `install_selftest_verdict` reads only between boot stages `t` and `e`, and `INSTOK`/`INSTFAIL` take precedence. `scheduler.c` `serial_print` still sends whole strings through `boot_serial_putc`. `core.sh test-shell` (`scripts/shell_test.py`) boots a `POPCORN_SERIAL_CONSOLE` build in which COM1 carries console text and keyboard input. | as cited | Multi-character text on debugcon can falsely satisfy the `'M'`/Phase-2 patterns. **New diagnostics (net tags, panic text) go to COM1 only** with a unique prefix (e.g. `NET:`) so they cannot be mistaken for shell output under test-shell, and the net test greps the serial log (§9.1). The existing hazard in `serial_print` should be fixed in the cleanup pass. |
| F9 | About 50 `static mut` globals in the Rust crate (plus 9 `fn ... -> &'static mut` accessors over them), and no lock type. | `drivers/{driver,irq,block/mod,block/virtio_blk}.rs`, `fs/fat32.rs`, `drivers/usb/xhci.rs`, etc. | New code adds **zero** `static mut` and uses `sync.rs`. |
| F10 | The PCI walk recurses through bridges and `bar_mem` sizes 64-bit BARs. `bar_io` returns the base only, with no size. There is no capability walk and no lock around CF8/CFC. | `drivers/bus/pci.rs` | Needed: a capability iterator (0x09 virtio vendor, 0x11 MSI-X), I/O BAR sizing, a CF8/CFC lock, and an ID table. |
| F11 | The Rust target is `x86_64-unknown-none` (soft-float), built with `opt-level = "s"`. There is no compiler-rt `memcpy`, so copies are byte loops. The C kernel, however, is compiled with SSE allowed (no `-mno-sse`), and `idt.asm` `irq_common`/`syscall_handler_asm` `fxsave64`/`fxrstor64` around every C call. | `scripts/lib/kernel.sh:67,70` (cargo), `:157–163` (C flags), `src/rust/Cargo.toml`, `alloc_shim.rs`, `src/core/idt.asm` | New vector stubs that call C/Rust (`vector_common` -> `vector_dispatch`) **must** `fxsave64`/`fxrstor64` like `irq_common`, because a handler may switch tasks. Hot paths need `#[inline]`. Any copy we keep (UDP payload) uses a `rep movsb` helper. |
| F12 | The GRUB run/smoke path uses `-cpu qemu64` on the default `pc` machine (`run_legacy_qemu` adds `-smp $QEMU_CORES`; `qemu_legacy_smoke` is single-CPU). UEFI runs `-machine q35 -cpu max`, and so does the test-shell GRUB boot (`qemu_legacy_shell_boot`). | `kernel.sh:294,320`; `qemu-uefi.sh` (`qemu_uefi_usb_args`, `qemu_uefi_run_interactive`, `qemu_legacy_shell_boot`) | xAPIC is always available. x2APIC is available only with `-cpu max`. MSI-X targets the BSP's APIC ID. The default NIC differs by machine (F6). |

---

## 2. Layout

```
src/rust/popcorn_kernel/src/
  sync.rs                     NEW  SpinLock<T>, IrqSpinLock<T> (pushfq; cli; ... popfq), Once<T>
  util/copy.rs                NEW  rep-movsb copy/fill; csum (u64 accumulate, fold)
  arch/x86/
    mod.rs  msr.rs            NEW  rdmsr/wrmsr (kernel-only)
    lapic.rs                  NEW  xAPIC (UC MmioRegion) | x2APIC (MSR) behind enum LapicIo
    vectors.rs                NEW  [VecSlot; 256] table, vector allocator, vector_dispatch()
  drivers/
    mod.rs                    CHG  + grant, queue, virtio, net; io becomes private to grant
    grant/                    NEW  the only code with raw hardware access
      mod.rs                       GrantId, GrantError, DeviceGrants (per-probe bundle)
      mmio.rs                      MmioRegion
      port.rs                      PortRange (io.rs asm moves here, private)
      irq.rs                       IrqHandle, IrqSlot
      dma.rs                       DmaPool, Dma<T>, DmaArray<T>, BufSlab, DeviceAddr, DmaSafe
    queue.rs                  NEW  Queue trait, Budget, Completion, QueueFull
    bus/pci.rs                CHG  PciFunction, caps iterator, I/O BAR sizing, CF8/CFC lock, ID table
    bus/msix.rs               NEW  MSI-X table programming (kernel-owned; never handed to a driver)
    virtio/                   NEW  shared by blk, net (later gpu/input/rng)
      mod.rs                       status + reserved feature bits, DeviceType, VirtioError, negotiate()
      transport.rs                 trait Transport
      modern.rs                    ModernTransport (cap 0x09; MmioRegion x4)
      legacy.rs                    LegacyTransport (PortRange; dynamic cfg_base 20/24)
      pci.rs                       discovery + transport selection
      virtqueue.rs                 SplitQueue<const N: usize>
      net.rs                       virtio-net driver
      blk.rs                       virtio-blk (M4; block/virtio_blk.rs becomes a re-export shim)
    net/mod.rs                NEW  NetDevice trait, NetCaps, NetStats, RxBatch, TxBuf, net registry
    net/loopback.rs           NEW  in-memory NetDevice for stack self-tests
    device.rs                 CHG  DeviceClass::Net
    registry.rs               CHG  catalog entry "net0"
  net/                        NEW  protocol stack = policy (bootstrap for the future *PopNet realm)
    mod.rs iface.rs pump.rs checksum.rs eth.rs arp.rs ipv4.rs icmp.rs udp.rs dhcp.rs dns.rs cmd.rs
src/core/irq_vectors.asm      NEW  %rep stubs 0x30..0xFF (minus 0x80) -> vector_common -> vector_dispatch
src/core/idt.c                CHG  catch-all fill before any vector is bound
src/core/kernel.c             CHG  rust_io_pump() next to timer_poll(); UEFI sti gated (Q1)
src/core/shell.c              CHG  net/ping/irq/pci -caps wrappers -> rust_net_cmd()
src/includes/rust_abi.h       CHG  rust_io_pump, rust_net_cmd, vector_dispatch prototypes
```

`net/` sits outside `drivers/` on purpose. Drivers are mechanism and the stack is policy, which matches the ExoCore direction in `roadmap-UTD.md` (Phase 0.2 `*PopNet`, Phase 26.2 "userspace network stack").

**Enforcement.** Visibility comes first: `io.rs` asm and `dma::{virt_to_phys, phys_to_virt, map_mmio}` become `pub(in crate::drivers::grant)`. NVMe, xHCI, kbd and serial have not migrated yet. They keep building through a `#[deprecated] pub use grant::port::raw as legacy_io`, which makes every remaining use show as a warning. A script check (`scripts/lib/lint.sh`) fails if `asm!`, `io::`, `inl(`, `outl(`, `virt_to_phys`, `phys_to_virt` or `map_mmio` appear under `drivers/virtio/`, `drivers/net/` or `net/`.

---

## 3. Framework

### 3.1 Grants (decision 1)

```rust
// drivers/grant/mod.rs
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub struct GrantId(u16);
#[derive(Debug)] pub enum GrantError { OutOfRange, Busy, NoVectors, NoMemory, Unsupported }

/// Everything a driver receives at probe. Nothing else gives hardware access.
pub struct DeviceGrants { pub id: GrantId, pub pci: PciFunction, pub name: &'static str }
```

#### 3.1.1 MmioRegion

```rust
pub struct MmioRegion { base: NonNull<u8>, len: usize, grant: GrantId }
impl MmioRegion {
    #[inline] pub fn r8(&self, off: usize) -> u8;   #[inline] pub fn w8(&self, off: usize, v: u8);
    #[inline] pub fn r16(&self, off: usize) -> u16; #[inline] pub fn w16(&self, off: usize, v: u16);
    #[inline] pub fn r32(&self, off: usize) -> u32; #[inline] pub fn w32(&self, off: usize, v: u32);
    #[inline] pub fn w64_split(&self, off: usize, v: u64); // lo then hi (virtio 64-bit fields)
    pub fn len(&self) -> usize;
    pub fn sub(&self, off: usize, len: usize) -> Result<MmioRegion, GrantError>; // narrow only
}
// Send, !Sync: the owning driver wraps it in its lock.
```

- Each access is `read_volatile`/`write_volatile` with a bounds check. Offsets are almost always constants, so the check folds away.
- An out-of-bounds access does **not panic**: it bumps `GRANT_VIOLATIONS`, returns all-ones and drops the write. A panic would currently look like a hang (F7).
- The back end is a new `dma::map_mmio_uc(phys, size)`. It always maps through the PML4[256] window with PWT\|PCD, even below 64 GiB (F3).
- The direct map still aliases those pages as WB. With firmware MTRRs marking the hole UC, the effective type is UC either way. Removing the alias properly means splitting the 2 MiB direct-map pages over MMIO holes (Q8).

#### 3.1.2 PortRange

```rust
pub struct PortRange { base: u16, len: u16, grant: GrantId }
impl PortRange {
    pub fn r8(&self, off: u16) -> u8;   pub fn w8(&self, off: u16, v: u8);
    pub fn r16(&self, off: u16) -> u16; pub fn w16(&self, off: u16, v: u16);
    pub fn r32(&self, off: u16) -> u32; pub fn w32(&self, off: u16, v: u32);
    pub fn sub(&self, off: u16, len: u16) -> Result<PortRange, GrantError>;
}
```

The length comes from a new I/O BAR size probe. With decode disabled: write `0xFFFF_FFFF`, read back, `size = (!(v & !0x3) + 1) & 0xFFFF`, then restore.

#### 3.1.3 PciFunction: the mediated grant source

```rust
pub struct PciFunction { addr: PciAddr, vendor: u16, device: u16, subsys_dev: u16, grant: GrantId }
impl PciFunction {
    pub fn ids(&self) -> (u16, u16);  pub fn subsystem_device(&self) -> u16;
    pub fn cfg_r8/16/32(&self, off: u8);                 // read-only general access
    pub fn caps(&self) -> CapIter<'_>;                   // status bit 4, ptr @0x34, loop guard 48
    pub fn vendor_cap(&self, cap_off: u8) -> [u8; 20];   // raw virtio_pci_cap bytes
    pub fn bar(&self, i: u8) -> Result<Bar, GrantError>; // refuses pages holding MSI-X table/PBA
    pub fn enable(&self, mem: bool, io: bool, bus_master: bool); // always sets INTx-disable (bit 10)
    pub fn disable_bus_master(&self);
    pub fn alloc_irqs(&self, want: u16, name: &'static str) -> IrqSet; // MSI-X, else Polled
}
pub enum Bar { Mmio(MmioRegion), Io(PortRange) }
pub fn claim(ids: &[(u16, u16)], subsys: Option<u16>, n: usize) -> Option<DeviceGrants>; // claim bitmap
```

Drivers **cannot**:
- write BARs,
- write the command register, except through `enable`,
- write MSI-X entries or arbitrary config space.

That is the line that later stops a ring-3 driver from injecting interrupts. All CF8/CFC pairs go through one `IrqSpinLock`, because the two-step access is not atomic once ISRs or tasks interleave.

Transitional IDs (0x1000..0x103F) are shared by every transitional virtio type, so `claim` also checks the subsystem device ID (net = 1).

#### 3.1.4 IrqHandle

```rust
pub struct IrqSlot {            // static [IrqSlot; 256] in arch/x86/vectors.rs
    pending: AtomicU32,         // ISR sets; consumer swaps to 0
    count:   AtomicU64,
    owner:   AtomicU16,         // GrantId index into the name table (not a &str: fat pointer)
}
pub enum IrqSource { MsiX { entry: u16 }, Polled }
pub struct IrqHandle { vector: u8, source: IrqSource, slot: &'static IrqSlot }
impl IrqHandle {
    pub fn is_polled(&self) -> bool;
    pub fn take(&self) -> bool;     // pending.swap(0, Acquire) != 0
    pub fn mask(&self); pub fn unmask(&self); // MSI-X vector-control bit 0, via kernel-owned table
    pub fn count(&self) -> u64;
}
pub struct IrqSet { pub config: IrqHandle, pub queues: [IrqHandle; 2] } // net: rx, tx
```

Plain MSI (cap 0x05) is not implemented now (Q10). Virtio only uses MSI-X.

#### 3.1.5 DmaPool and buffers

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)] #[repr(transparent)]
pub struct DeviceAddr(u64);           // what the device sees. Today == phys; IOVA under VT-d.
                                      // Deliberately no conversion to a pointer.
pub unsafe trait DmaSafe: Copy + 'static {}   // repr(C), every bit pattern valid

pub struct DmaPool { grant: GrantId, chunks: [Option<Chunk>; 32], pages: u32, limit: u32, poisoned: bool }
impl DmaPool {
    pub fn new(g: &DeviceGrants, page_limit: u32) -> Self;
    pub fn alloc<T: DmaSafe>(&mut self) -> Result<Dma<T>, GrantError>;                 // zeroed
    pub fn alloc_array<T: DmaSafe>(&mut self, n: usize, align: usize) -> Result<DmaArray<T>, GrantError>;
    pub fn slab(&mut self, chunk: u16, count: u16) -> Result<BufSlab, GrantError>;
    pub fn poison(&mut self);          // device may still DMA: leak on drop instead of freeing
    pub fn outstanding_pages(&self) -> u32;
}
pub struct BufSlab { /* chunk table: base VA + DeviceAddr per 64 KiB run */ }
impl BufSlab {
    #[inline] pub fn addr(&self, id: u16) -> DeviceAddr; // chunk[id >> 5].dev + (id & 31) << 11
    #[inline] pub fn bytes(&self, id: u16) -> &[u8];
    #[inline] pub fn bytes_mut(&mut self, id: u16) -> &mut [u8];
}
```

Rules:
- **Pools come from 64 KiB runs** (16 pages), each one `kmalloc` and one `mem_block`.
  - A 1 MiB net pool costs 16 blocks and needs only 16 contiguous frames per run.
  - A single 1 MiB arena would ask `pmm_alloc_contig` for 256 contiguous frames (rejected).
  - Page-by-page allocation would burn 256 of the 2048 slots (rejected).
- Ring memory needs at most 3 contiguous pages per queue.
- 2 KiB slots never cross a page, so every buffer is a single physically contiguous descriptor.
- x86 DMA is cache-coherent. `sync_for_device`/`sync_for_cpu` exist as inline no-ops, so the API stays honest for a future IOMMU.
- **Teardown:** a pool may be freed only after the device has been reset or bus mastering disabled. Otherwise it is poisoned and leaked. This formalises today's `mem::forget` in `virtio_blk.rs`.
- Slot free lists are fixed `[u16; N]` stacks, never `Vec`.

#### 3.1.6 Path to ring 3 (UTD Phases 12.4, 13, 22–25)

| Grant | Ring 0 now | Ring-3 realm later |
|---|---|---|
| MmioRegion | kernel VA, UC | `CAP_MMIO`: same pages mapped UC\|USER. Pages holding the MSI-X table/PBA are never mapped. |
| PortRange | `in`/`out` | `CAP_IOPORT`: needs a TSS I/O bitmap. Today `idt.c` puts the IOPB base past the TSS limit. Modern virtio needs no PIO. |
| IrqHandle | atomic slot | `*PopEvent` bound to the vector. Mask/unmask become syscalls. The table stays kernel-owned. |
| DmaPool | `alloc_pages`, DeviceAddr = phys | `*PopMem` DMA-suitable pinned pages. DeviceAddr = IOVA once VT-d exists (Phase 25). |
| PciFunction | direct calls | `CAP_DEVICE`, with the same methods as syscalls |

Stated plainly: without an IOMMU, a ring-3 driver with bus mastering can still DMA anywhere. Ring 3 buys fault isolation and restart first. VT-d buys security later.

### 3.2 The universal queue (decision 2)

```rust
// drivers/queue.rs
#[derive(Clone, Copy)] pub struct Budget(pub u16);
#[derive(Clone, Copy)] pub enum QStatus { Ok, Err(u16) }
#[derive(Clone, Copy)] pub struct Completion<C: Copy> { pub cookie: C, pub len: u32, pub status: QStatus }
pub struct QueueFull;

/// Scatter segment; lives on the caller's stack ([Seg; 1] net, [Seg; 3] blk).
#[derive(Clone, Copy)] pub struct Seg { pub addr: DeviceAddr, pub len: u32, pub device_writes: bool }

pub trait Queue {
    type Cookie: Copy;                              // u16 slot id for net, request tag for blk
    fn capacity(&self) -> u16;
    fn free_slots(&self) -> u16;
    /// Stage one request. Never rings the doorbell. Never allocates.
    fn submit(&mut self, segs: &[Seg], cookie: Self::Cookie) -> Result<(), QueueFull>;
    /// Publish everything staged with one index store; notify only if the device wants it.
    /// Returns true if the doorbell was written.
    fn kick(&mut self) -> bool;
    /// Deliver at most `budget` completions. Generic => monomorphised, no dyn per completion.
    fn reap<F: FnMut(Completion<Self::Cookie>)>(&mut self, budget: Budget, f: F) -> u16 where Self: Sized;
    /// Request a completion interrupt, then re-check. true = work raced in, reap again.
    fn arm(&mut self) -> bool;
    fn disarm(&mut self);
}
```

The hot path makes no allocations:
- Cookies are `Copy` and stored in a fixed `[DescState; N]` array.
- Requests are borrowed `&[Seg]`.
- Queue depth is a const generic.
- `dyn` dispatch happens once per batch at the class boundary (`dyn NetDevice`), never per descriptor.

The same shape covers the other device families:
- **virtio:** desc + avail is submit, the notify write is kick, the used ring is reap.
- **NVMe** (`block/nvme.rs`): SQ entry is submit, the SQ tail doorbell is kick, the CQ phase bit is reap, with one CQ head write per batch.
- **xHCI** (`usb/xhci.rs`): a TRB with the cycle bit is submit, the doorbell is kick, the event ring is reap.

Synchronous helpers (`submit_and_wait`) sit on top, so `block/mod.rs` `raw_read`/`raw_write` keep their blocking API.

### 3.3 Interrupts: IDT, LAPIC, MSI-X, hybrid flow (decision 3)

#### 3.3.1 IDT and vector map

| Range | Use |
|---|---|
| 0x00–0x1F | exceptions (existing) |
| 0x20–0x2F | 8259 PIC (existing IRQ0/1; the rest go to a catch-all) |
| 0x30–0x3F | reserved for the kernel: LAPIC timer, IPIs |
| 0x40–0xEF | **device pool** (bitmap allocator under `IrqSpinLock`), **excluding 0x80** |
| 0x80 | syscall gate, DPL3. **Never overwritten.** |
| 0xFE | LAPIC error |
| 0xFF | spurious: `iretq`, **no EOI** |

- `irq_vectors.asm` generates stubs for every vector 0x30..0xFF except 0x80 with `%rep` (0x20–0x2F already have PIC stubs in `idt.asm`/`irq.c` since the 2026-11-22 fixes). Each stub is `push imm; jmp vector_common`.
- `vector_common` saves the 15 GPRs like `timer_handler`, runs `cld`, then `sub rsp, 8`. The CPU frame (40 B) + vector (8) + 15 regs (120) leaves RSP ≡ 8 mod 16, and the `sub` realigns it for the call. It then calls `vector_dispatch(vec)`.
- C is built with SSE allowed (F11), so `vector_common` must `fxsave`/`fxrstor` (or the kernel must move to `-mgeneral-regs-only`; see open questions).
- **M0 fills every not-yet-bound vector with these stubs.** Unbound vectors route to a catch-all that:
  - bumps `IrqSlot[v].count`;
  - for 0x20–0x2F, keeps using the existing PIC path in `irq.c` (EOI + spurious 7/15 check);
  - for vectors ≥ 0x30, does a LAPIC EOI if the LAPIC is enabled and the vector's ISR bit is set;
  - returns.
- Binding a device vector only swaps the handler in the Rust table. The IDT gate never changes again.

#### 3.3.2 LAPIC (`arch/x86/lapic.rs`)

1. Require CPUID.1:EDX[9]. Otherwise everything stays `Polled`.
2. Read `rdmsr(0x1B)`.
   - Base = `val & 0x000F_FFFF_FFFF_F000` (expect 0xFEE00000).
   - Bit 11 = global enable (set it if clear). Bit 10 = x2APIC.
3. Choose the access mode.
   - If x2APIC is already enabled, use MSRs: ID `0x802`, TPR `0x808`, EOI `0x80B`, SVR `0x80F`, ICR `0x830`, SELF_IPI `0x83F`.
   - Otherwise use xAPIC over a UC `MmioRegion`: ID `0x20` (bits 31:24), TPR `0x80`, EOI `0xB0`, SVR `0xF0`, ISR `0x100..0x170`, ESR `0x280`, ICR `0x300/0x310`, LVT timer `0x320`, LINT0 `0x350`, LINT1 `0x360`, LVT error `0x370`.
4. `TPR = 0`, `SVR = 0x100 | 0xFF`, `LVT_ERROR = 0xFE`, `LVT_TIMER = 1<<16` (masked).
   - Masking the timer also covers a LAPIC timer that firmware left armed.
5. **GRUB path:** program `LINT0 = 0x700` (ExtINT, virtual wire) and `LINT1 = 0x400` (NMI) explicitly, so PIC IRQ0/1 keep arriving through `irq.c`. Do not rely on firmware defaults.
6. **UEFI path (Q1):**
   - mask both PICs (`0xFF` to 0x21/0xA1) and mask LINT0;
   - make sure the catch-all fill (§3.3.1) is in place;
   - then `sti`. Only LAPIC/MSI vectors are live.
   - If the hang persists, UEFI stays polled. Decision 3 allows that as the fallback.
7. Self-test before any device: send a self-IPI on vector 0x40, using ICR shorthand `0b01 << 18` (or the SELF_IPI MSR). Expect `IrqSlot[0x40].count == 1`. This proves the stubs, IDT, LAPIC delivery and EOI without a NIC.

LAPIC code lives in Rust (not `src/core/apic.c`), next to the `IrqHandle` grant layer that consumes it (Q5).

#### 3.3.3 MSI-X (`bus/msix.rs`, inside the grant layer)

1. Find cap 0x11.
   - Message Control at cap+2: table size = `(mc & 0x7FF) + 1`, bit 14 = function mask, bit 15 = enable.
   - Table at cap+4 (BIR = bits 2:0, offset = rest). PBA at cap+8.
   - On QEMU virtio-pci: the MSI-X table is in **BAR1**, the modern caps in **BAR4**, legacy I/O in BAR0.
2. Map `bar(BIR).sub(off, 16·size)`. The mapping stays kernel-owned.
3. Set `mc |= ENABLE | FUNCTION_MASK` and set command bit 10 (INTx disable).
4. For each entry i (0 = config, 1 = rx, 2 = tx), allocate a vector v and write:
   - `addr_lo = 0xFEE0_0000 | (bsp_apic_id << 12)` (physical destination, RH = 0);
   - `addr_hi = 0`;
   - `data = v` (fixed delivery, edge);
   - `vector_ctrl = 1` (masked).
5. Clear FUNCTION_MASK. Each entry is unmasked individually when its queue arms for the first time. In poll mode the entries stay **masked**, so QEMU does no interrupt-injection work.

#### 3.3.4 ISR and pump

```rust
#[no_mangle] pub extern "C" fn vector_dispatch(vec: u64) {
    let s = &SLOTS[vec as usize];
    s.count.fetch_add(1, Relaxed);
    s.pending.store(1, Release);     // the ISR never touches rings or driver state
    lapic::eoi();                    // LAPIC EOI only; never pic_send_eoi for >= 0x30
}
```

The pump is `rust_io_pump()`, called from kmain next to `timer_poll()`:

```
for each net device (try_lock; never spin from the loop):
    if rx.take() || rx.is_polled() || dev.more_pending {
        rxq.disarm()
        n = dev.rx_batch -> stack -> rx_recycle (budget 64)   // §6
        dev.more_pending = (n == 64)
        if !dev.more_pending && !rx.is_polled() && rxq.arm() { dev.more_pending = true } // raced
    }
    dev.reap_tx(budget 64); stack timers (ARP/DHCP/DNS); dev.tx_flush()   // one TX kick per pass
```

- **GRUB, IRQ mode:** idle uses `cli; if nothing pending { sti; hlt } else { sti }`. The one-instruction shadow after `sti` makes `sti; hlt` safe against lost wakeups.
- **UEFI or `vectors=0`:** the pump runs every loop iteration. An idle pass costs one acquire load of `used.idx` per queue in RAM, with no MMIO exit.
- **TX completion interrupts are permanently off.** TX is reaped in every pump and whenever `tx_alloc` finds the slab empty. The TX vector exists but stays masked, so it can be enabled later (via `arm_after(n)`) if measurements justify it.

### 3.4 Locking

```rust
pub struct SpinLock<T>    { locked: AtomicBool, v: UnsafeCell<T> }   // test-and-test-and-set
pub struct IrqSpinLock<T> { inner: SpinLock<T> }                     // saves RFLAGS, cli; restores on Drop
impl<T> IrqSpinLock<T> { pub const fn new(v: T) -> Self; pub fn lock(&self) -> IrqGuard<'_, T>;
                          pub fn try_lock(&self) -> Option<IrqGuard<'_, T>>; }
pub struct Once<T>;
```

- **Why disable interrupts even though the ISR touches no driver state:** on GRUB, IRQ0 can enter the scheduler (`irq_quiet_*` in `irq.c` exists for this). A task preempted while holding a plain spinlock on one CPU would let the next task spin forever.
- Critical sections are bounded by the budget. Locks are taken once per pump pass or per shell-command step, never per packet, and never across a `timer_poll()` wait.
- Device state lives in `static NET0: IrqSpinLock<Option<Iface<VirtioNet>>>`. `static NET_PRESENT: AtomicBool` makes the no-NIC pump cost one load.
- ISR-shared data is atomics only. Lock order: `NET0` → PCI cfg lock → vector allocator.
- SMP later means one queue has one owner CPU (NAPI semantics). The shape does not change.
- New modules use `#![deny(static_mut_refs)]`. Converting the existing 67 globals is separate cleanup (Q9).

---

## 4. Shared virtio transport

### 4.1 Trait and negotiation

```rust
pub struct QueueAddrs { pub desc: DeviceAddr, pub driver: DeviceAddr, pub device: DeviceAddr }
pub trait Transport: Send {
    fn device_type(&self) -> DeviceType;   fn is_modern(&self) -> bool;
    fn reset(&mut self);                   // write 0, wait until status reads 0 (tick timeout)
    fn status(&self) -> u8;                fn add_status(&mut self, bits: u8);
    fn device_features(&mut self) -> u64;  // legacy: low 32 bits only
    fn set_driver_features(&mut self, f: u64);
    fn queue_max(&mut self, q: u16) -> u16;                       // 0 = no such queue
    fn setup_queue(&mut self, q: u16, size: u16, a: &QueueAddrs, vec: Option<u16>) -> Result<(), VirtioError>;
    fn enable_queue(&mut self, q: u16);    // modern: queue_enable = 1; legacy: no-op (PFN enabled it)
    fn notifier(&self, q: u16) -> Notifier;                       // precomputed doorbell
    fn set_config_vector(&mut self, v: Option<u16>) -> Result<(), VirtioError>; // readback != 0xFFFF
    fn read_config(&self, off: usize, out: &mut [u8]);            // modern: generation loop
}
pub fn negotiate(t: &mut dyn Transport, wanted: u64, required: u64) -> Result<u64, VirtioError>;
// reset -> ACKNOWLEDGE(1) -> DRIVER(2) -> f = dev & wanted; required ⊄ f => FAILED(128)
// -> write f -> modern: FEATURES_OK(8), re-read, must persist -> f
```

Status bits are ACKNOWLEDGE 1, DRIVER 2, DRIVER_OK 4, FEATURES_OK 8, DEVICE_NEEDS_RESET 64, FAILED 128. Every status read in the pump checks NEEDS_RESET and FAILED.

### 4.2 Modern PCI transport (`virtio/modern.rs`), used by net from M1

- Walk vendor caps (ID 0x09). `virtio_pci_cap` is: `cap_vndr u8, cap_next u8, cap_len u8, cfg_type u8, bar u8, id u8, pad[2], offset le32, length le32`.
- cfg_type values:
  - 1 = COMMON
  - 2 = NOTIFY (`notify_off_multiplier le32` at cap+16)
  - 3 = ISR (never read with MSI-X)
  - 4 = DEVICE
  - 5 = PCI_CFG (ignored)
- Take the first cap of each type on a memory BAR and map it with `pci.bar(i)?.sub(offset, length)`. OVMF may put the 64-bit BAR4 above 512 GiB; the high window covers that.

Common config:

| Off | Field | Off | Field |
|---|---|---|---|
| 0x00 | device_feature_select le32 | 0x16 | queue_select le16 |
| 0x04 | device_feature le32 | 0x18 | queue_size le16 |
| 0x08 | driver_feature_select le32 | 0x1A | queue_msix_vector le16 |
| 0x0C | driver_feature le32 | 0x1C | queue_enable le16 |
| 0x10 | config_msix_vector le16 | 0x1E | queue_notify_off le16 |
| 0x12 | num_queues le16 | 0x20 | queue_desc le64 |
| 0x14 | device_status u8 | 0x28 | queue_driver le64 |
| 0x15 | config_generation u8 | 0x30 | queue_device le64 |

- 64-bit features are read and written as two 32-bit halves through the select registers.
- The doorbell for queue q is at `notify_base + queue_notify_off(q) * notify_off_multiplier`. It takes a 16-bit write of q (NOTIFICATION_DATA is refused). The address is precomputed once per queue.
- The driver may shrink `queue_size` (power of two).
- Device config is read in a loop: read `config_generation`, read the fields, re-read `config_generation`, and retry if it changed.

### 4.3 Legacy transport (`virtio/legacy.rs`), used by blk until M4 and optionally by net (Q2)

| Off | Register | Off | Register |
|---|---|---|---|
| 0 | device_features le32 | 16 | queue_notify le16 |
| 4 | guest_features le32 | 18 | status u8 |
| 8 | queue_pfn le32 (addr >> 12) | 19 | isr u8 |
| 12 | queue_size le16 (read-only) | 20 | config_msix_vector le16 *(only with MSI-X enabled)* |
| 14 | queue_select le16 | 22 | queue_msix_vector le16 *(only with MSI-X enabled)* |

- Device config starts at **20, or at 24 when MSI-X is enabled** in the PCI capability. `cfg_base()` is therefore dynamic, and MSI-X must be enabled **before** any device-config read. The current `REG_CONFIG = 20` breaks as soon as MSI-X is turned on (F2).
- There is no FEATURES_OK and no feature bits ≥ 32.
- Queue size is fixed by the device. The ring is one contiguous run: desc, then avail, then used aligned to 4096 (`legacy_layout(n)` moves out of `virtio_blk.rs`). The PFN register is 32 bits, so the ring must sit below 2^44 (always true here).

### 4.4 Selection (`virtio/pci.rs`)

- IDs: transitional 0x1000 (net, subsystem 1) and 0x1001 (blk, subsystem 2); modern 0x1041 (net) and 0x1042 (blk).
- If COMMON, NOTIFY and DEVICE caps exist and map, use modern. Otherwise, if BAR0 is I/O, use legacy. A `virtio=legacy|modern` override exists for the test matrix.
- QEMU detail: a device on q35's root bus `pcie.0` keeps its legacy BAR by default. Behind a `pcie-root-port` it defaults to `disable-legacy=on`.

### 4.5 SplitQueue (`virtio/virtqueue.rs`)

```rust
#[repr(C)] #[derive(Clone, Copy)] struct Desc { addr: u64, len: u32, flags: u16, next: u16 } // F_NEXT 1, F_WRITE 2
// avail: flags le16, idx le16, ring[N] le16, used_event le16       (6 + 2N)
// used : flags le16, idx le16, ring[N] {id le32, len le32}, avail_event le16  (6 + 8N)
pub struct SplitQueue<const N: usize> {
    desc: DmaArray<Desc>, avail: Dma<..>, used: Dma<..>,   // modern: three separate pages
    notify: Notifier, qidx: u16,
    free_head: u16, num_free: u16,
    avail_shadow: u16,     // next avail.idx; never read back from shared memory
    kicked_at: u16,        // avail idx at last notify
    last_used: u16,
    state: [DescState; N], // cookie + next_free + chain_len: driver-private ("desc_extra")
    event_idx: bool, stats: QueueStats,
}
```

- **The free list lives in `state[i].next_free`, never in device-visible `desc.next`.** The driver never trusts or reads back device-shared descriptor memory, which also hardens it for a ring-3 realm.
- Under modern, desc, avail and used each get their own page, so the line QEMU writes (`used.*`) is never the line the guest writes (no false sharing). Legacy has to use the contiguous layout.
- Reject `N == 0`. The spec maximum is 32768; we use `min(max, 256)` on modern.

**Fences (x86 TSO, coherent DMA):**

| Point | Barrier |
|---|---|
| desc/avail.ring writes → `avail.idx` store | `AtomicU16::store(Release)` (plain `mov` + compiler fence) |
| `avail.idx` store → read `used.flags` / `avail_event` (kick decision) | **full**: `fence(SeqCst)` (`mfence`; a `lock or [rsp]` variant needs inline asm, measure later) |
| read `used.idx` → read `used.ring[]` and payload | `AtomicU16::load(Acquire)` |
| write `used_event` / clear NO_INTERRUPT (arm) → re-read `used.idx` | **full** |
| last ring write → notify MMIO (UC) | none; a UC store is ordered after earlier WB stores |

That is two full barriers per batch, against one or more per spin iteration in today's `virtio_blk.rs`.

**Notification suppression:**
- **Without EVENT_IDX (M1):**
  - kick only if `used.flags & VIRTQ_USED_F_NO_NOTIFY == 0`;
  - disarm sets `avail.flags = VIRTQ_AVAIL_F_NO_INTERRUPT` (1), arm clears it, then fence and re-check `used.idx != last_used`.
- **With EVENT_IDX (M2):**
  - The spec requires `avail.flags = 0` and the device ignores it.
  - Kick if `vring_need_event(avail_event, avail_shadow, kicked_at)`, i.e. `(u16)(new − event − 1) < (u16)(new − old)`.
  - Arm writes `used_event = last_used`, fences, and re-checks.
  - Disarm leaves `used_event` stale, which makes device interrupts one-shot by construction. A stale value can be crossed again after 2^16 completions, so disarm writes `used_event = last_used − 1 − N/2`, and anything left is a harmless spurious interrupt.
- `reap` takes one acquire snapshot of `used.idx` and processes `min(snapshot − last_used, budget)` entries with no further shared reads.

---

## 5. virtio-net

### 5.1 Features

The bits match VIRTIO 1.2 §5.1.3 and §6. "Offered by QEMU" means `-netdev user`, virtio-net-pci, current QEMU.

| Bit | Name | Decision | Why |
|---|---|---|---|
| 32 | VERSION_1 | **require** (modern) | 12-byte header, LE, FEATURES_OK, implies any-layout |
| 5 | MAC | accept | MAC from config +0. If absent, generate locally administered `52:54:00:xx:xx:xx` and log it. |
| 16 | STATUS | accept | link bit (`S_LINK_UP` = 1) at config +6; config vector → `link_up` |
| 3 | MTU | accept if offered | read `mtu` at +10; refuse to init if > 1500 until jumbo support |
| 63 | SPEED_DUPLEX | accept if offered | speed le32 at +12, duplex u8 at +16, shown in `net -info` |
| 29 | RING_EVENT_IDX | refuse in M1, **accept from M2** | one-shot interrupts and kick suppression, added once the basic path is proven |
| 33 | ACCESS_PLATFORM | accept **only if offered** | identity translation with no IOMMU is correct. QEMU offers it only with `iommu_platform=on`. |
| 36 | ORDER_PLATFORM | accept only if offered | x86 TSO + coherent DMA: §4.5 fences already meet platform ordering |
| 0 / 1 | CSUM / GUEST_CSUM | accept if offered (M3+), **software path is primary** | QEMU clears both (and all TSO/UFO/USO bits) when the peer has no vnet header, and slirp has none. They are only testable on `tap,vnet_hdr=on`. |
| 15 | MRG_RXBUF | refuse | with VERSION_1 the header is 12 B anyway. Without MRG_RXBUF and GUEST_TSO/UFO, the spec requires RX buffers ≥ 1526 B; ours give 2010 B. Revisit with TSO. |
| 7–14 | GUEST_/HOST_ TSO4, TSO6, ECN, UFO | refuse | need 65562 B RX buffers or GSO; no TCP yet |
| 54, 55, 56 | GUEST_USO4/6, HOST_USO | refuse | same |
| 2, 17–23 | CTRL_GUEST_OFFLOADS, CTRL_VQ, CTRL_RX, CTRL_VLAN, CTRL_RX_EXTRA (20), GUEST_ANNOUNCE, MQ, CTRL_MAC_ADDR | refuse | no control queue, so queue 2 is never set up. Default device filtering is enough. |
| 52, 53, 57, 59–62 | VQ_NOTF_COAL, NOTF_COAL, HASH_REPORT, GUEST_HDRLEN, RSS, RSC_EXT, STANDBY | refuse | single CPU, no ctrl vq |
| 28 | INDIRECT_DESC | refuse | one descriptor per packet |
| 34, 35 | RING_PACKED, IN_ORDER | refuse | split ring first; both are later perf experiments (Q6) |
| 37–40 | SR_IOV, NOTIFICATION_DATA, NOTIF_CONFIG_DATA, RING_RESET | refuse | not needed |
| legacy 27 / 24 / 6 | ANY_LAYOUT / NOTIFY_ON_EMPTY / GSO | only on the legacy path (Q2) | legacy without ANY_LAYOUT must split header and frame into 2 descriptors on **both** RX and TX |

`NET_WANTED` and `NET_REQUIRED` live in `virtio/mod.rs`. Probe logs `net0: feat dev=<64-bit hex> acked=<hex>`, so offload availability is visible, not assumed.

**Header:** `virtio_net_hdr { flags u8, gso_type u8, hdr_len le16, gso_size le16, csum_start le16, csum_offset le16, num_buffers le16 }` is 12 B with VERSION_1. On legacy without MRG_RXBUF it is 10 B, because `num_buffers` is absent. `hdr_len` is a field set after negotiation, never a constant. The TX header is all zeros (GSO_NONE, no NEEDS_CSUM) unless CSUM was acked.

### 5.2 Queues and memory

- Queue 0 is receiveq1 and queue 1 is transmitq1. Size is `min(queue_max, 256)`.
- One `DmaPool` per NIC:
  - rings: 3 pages per queue × 2;
  - RX slab: 256 × 2 KiB = 512 KiB;
  - TX slab: 256 × 2 KiB = 512 KiB.
- Total is about 1 MiB in 16 runs of 64 KiB (Q7).

### 5.3 Probe sequence (modern), with the reviewers' order fixes applied

```
 1. g = pci::claim(&[(0x1AF4,0x1041),(0x1AF4,0x1000)], Some(1), 0)   else log "net: none", Ok(None)
 2. g.pci.enable(mem=true, io=false, bus_master=false)               (+INTx disable)
 3. t = virtio::pci::open(&g)?; t.reset()                            (wait for 0: OVMF VirtioNetDxe may have left it live)
 4. g.pci.enable(.., bus_master=true)                                 only after reset
 5. acked = negotiate(t, NET_WANTED, NET_REQUIRED)?                   (FEATURES_OK must stick)
 6. irqs = g.pci.alloc_irqs(3, "net0")                               M1: all Polled. M2: MSI-X enabled here,
                                                                      before any device-config read (legacy 20->24)
 7. read config (generation loop): mac, status, mtu
 8. t.set_config_vector(irqs.config)                                 readback 0xFFFF -> that source Polled
 9. for q in [0 rx, 1 tx]: size, setup_queue(q, n, addrs, irqs.queues[q]) (+readback), enable_queue(q)
10. post all RX buffers into avail (no notify)
11. t.add_status(DRIVER_OK)
12. kick RX                                                          MUST NOT notify before DRIVER_OK (spec §3.1.1)
13. register: DeviceClass::Net, catalog::publish_device("net0","net",0); log mac/feat/link/irq mode
on any error: add_status(FAILED); disable_bus_master(); poison pool if anything was posted
```

### 5.4 RX lifecycle (zero-copy, lent for one batch)

- **Post.** Slot `id` is posted at `addr(id) + 38`, len 2010, F_WRITE, cookie `id`.
  - The 12-byte header lands at [38, 50) and Ethernet at [50, 64).
  - **The IPv4 header then starts at offset 64, cache-line aligned.**
- **Reap** `(id, len)`.
  - Require `hdr_len + 14 ≤ len ≤ hdr_len + 1518`; otherwise `rx_err`.
  - Header flags give `DATA_VALID` → `CsumState::Valid` (only if GUEST_CSUM was acked).
  - `num_buffers` must be 1.
- **Lend.** `rx_batch` fills an `RxBatch` with up to 32 ids, all under the device lock. The stack reads frames through the batch, which borrows the `&'static` slab, **not** the device. So the stack can `tx_alloc`/`tx_submit` replies (ARP, echo) on the same device while it holds the batch. Anything the stack keeps (a UDP payload into a socket ring) it copies with `util::copy`.
- **Recycle.** `rx_recycle(batch)` reposts all ids and kicks RX once.
- Invariant: `rx_posted + rx_lent == RX_SLOTS`. It is checked in `net -stats` and in tests.

### 5.5 TX lifecycle

- `tx_alloc()` pops a free slot from the TX stack. If the stack is empty, it reaps TX once (budget 64); if still empty it returns `None` and bumps `tx_full`.
- The stack writes the frame at headroom offset 64 and builds headers downward with `push_front`. The driver finally prepends the 12-byte header.
- `tx_submit(buf, meta)` posts one descriptor of `hdr_len + frame` bytes. Frames shorter than 60 B are zero-padded (harmless on virtio, correct for real NICs later).
- `tx_flush()` sends one kick per pump pass or per burst.
- TX reap returns slots. Buffers are never freed while in the ring.
- `net -down` order: reset the device, then free the pool.

---

## 6. Net-stack boundary

```rust
// drivers/net/mod.rs
#[derive(Clone, Copy)] pub struct NetCaps { pub tx_csum: bool, pub rx_csum: bool, pub mtu: u16 }
#[derive(Clone, Copy)] pub enum CsumState { Unknown, Valid }
#[derive(Clone, Copy)] pub struct TxMeta { pub csum: Option<(u16 /*start*/, u16 /*offset*/)> }
pub struct TxBuf { id: u16, head: u16, tail: u16 }          // HEADROOM = 64
pub struct RxBatch { slab: &'static BufSlab, ids: [u16; 32], lens: [u16; 32], csum: [CsumState; 32], n: u8 }
impl RxBatch { pub fn len(&self) -> usize; pub fn frame(&self, i: usize) -> (&[u8], CsumState); }

pub trait NetDevice: Send {
    fn name(&self) -> &'static str;
    fn mac(&self) -> [u8; 6];
    fn caps(&self) -> NetCaps;
    fn link_up(&self) -> bool;
    fn rx_batch(&mut self, budget: Budget, out: &mut RxBatch) -> usize;
    fn rx_recycle(&mut self, b: &mut RxBatch);               // repost + one RX kick
    fn tx_alloc(&mut self) -> Option<TxBuf>;
    fn tx_buf_mut(&mut self, b: &mut TxBuf) -> &mut TxFrame<'_>; // payload_mut / push_front
    fn tx_submit(&mut self, b: TxBuf, meta: TxMeta) -> Result<(), TxBuf>;
    fn tx_flush(&mut self);                                  // one TX doorbell
    fn reap_tx(&mut self, budget: Budget) -> usize;
    fn arm(&mut self) -> bool; fn disarm(&mut self);
    fn stats(&self) -> NetStats;
}
```

- The token / borrowed-RX / flush shape maps onto smoltcp's `phy::Device`, so the TCP decision can add an adapter without touching drivers.
- Later the same trait becomes two shared-memory rings of `u16` slot ids for a ring-3 `*PopNet` realm. That is decision 2 again.

Stack (`src/net/`; everything fixed-size, no heap after init):

| Module | Content |
|---|---|
| `iface.rs` | `Iface<D> { dev, mac, ip: Option<Ipv4Cidr>, gw, dns, arp: ArpCache<16>, pending: [Option<(TxBuf, Ipv4)>; 8], udp: [UdpSock; 16], ip_id }` |
| `eth.rs` | accept our MAC and broadcast; EtherType 0x0806 / 0x0800; count everything else |
| `arp.rs` | reply to requests for our IP; resolve next hop; hold up to 8 TxBufs while pending (1 s retry × 3, then drop); TTL in ticks; gratuitous ARP after config |
| `ipv4.rs` | version 4, IHL ≥ 5 (options skipped), header checksum always in software, total length ≤ frame; **drop fragments** (MF or offset) and count them; TX TTL 64, DF; on-link or via gw |
| `icmp.rs` | echo reply; echo client with id/seq table; RTT from TSC (`cpuinfo.rs` frequency), falling back to 10 ms ticks |
| `udp.rs` | pseudo-header checksum (skip verify if `Valid`); `bind/send_to`; 32-datagram ring per socket; echo service |
| `dhcp.rs` | DISCOVER/OFFER/REQUEST/ACK, BOOTP broadcast flag 0x8000, options 1/3/6/51/53/54/55, retransmit with backoff, renew at T1; fallback static 10.0.2.15/24 gw 10.0.2.2 dns 10.0.2.3 after 3 s (Q12) |
| `dns.rs` | single-question A query to `iface.dns` over UDP 53; id match, 2 s timeout × 2 |
| `checksum.rs` | RFC 1071, u64 accumulate and fold; host-tested |
| `pump.rs` | the §3.3.4 loop body |

slirp facts the tests rely on:
- Guest address 10.0.2.15; gateway/DHCP 10.0.2.2; DNS 10.0.2.3.
- The gateway answers ARP with MAC **52:55:0a:00:02:02**.
- Echo to 10.0.2.2 is answered inside libslirp, with no host ICMP privilege needed.

---

## 7. Shell commands

All commands are thin C wrappers in `shell.c` calling one FFI entry, `rust_net_cmd(argv, buf, len)`. They follow the `disk` conventions: dashed args, `available_commands[]` entries, a help block.

| Command | Action |
|---|---|
| `net` / `net -help` | usage |
| `net -info` | driver, PCI bdf, transport, MAC, link, MTU, offered/acked feature words, queue sizes, IRQ mode (msix vectors / poll), IP/gw/dns |
| `net -stats` | rx/tx pkts+bytes, errors/drops by reason, `tx_full`, irqs (+redundant), polls, budget-exhausted, kicks issued/suppressed, `rx_posted`/`tx_free` (leak check), avg batch |
| `net -up [A.B.C.D/NN gw G]` / `net -down` | configure (default static 10.0.2.15/24 gw 10.0.2.2) / reset device, free pool, assert `outstanding_pages == 0` |
| `net -dhcp` (M3) | run DHCP, apply lease |
| `net -arp [-flush]` | ARP cache |
| `ping <ip> [count] [size]` | default 4 × 56 B, 1 s timeout, count ≤ 100; **pumps `timer_poll()` + net itself** (F5) |
| `udp -send <ip> <port> <text>` / `udp -echo <port>` (M3) | UDP tests |
| `nslookup <name>` (M3) | DNS A query |
| `net -poll on\|off` | force mode for A/B runs |
| `irq` | vector table: vector, owner, source (pic/msix/catch-all), count, masked |
| `pci -caps` | extends `scan_bus0_print` with capability lists (MSI-X size, virtio cfg types) |

`dev` and `drive -info net0` show the node through the existing registry. Boot behaviour: probe at `init_drives` (non-fatal), configure only on `net -up`, auto-run under the selftest build (Q3).

---

## 8. Milestones (exact files)

Each milestone ends green on the existing `test-uefi` stability, debugcon and #PF tests.

### M0: framework and virtqueue extraction (no behaviour change)

| Files | Change |
|---|---|
| `src/rust/popcorn_kernel/src/lib.rs` | panic handler prints `PanicInfo` to **COM1 only** |
| `src/core/multiboot2.c`, `src/includes/boot_fb.h` | add `com1_putc`/`com1_puts` (COM1-only), export to Rust |
| `src/rust/popcorn_kernel/src/sync.rs`, `util/copy.rs` | new |
| `drivers/grant/{mod,mmio,port,irq,dma}.rs` | new; `drivers/io.rs` asm moves into `grant/port.rs` |
| `drivers/dma.rs` | `map_mmio_uc`; visibility restricted to `grant` |
| `drivers/queue.rs` | new |
| `drivers/bus/pci.rs` | `PciFunction`, `claim`, caps iterator, I/O BAR sizing, CF8/CFC lock, net IDs + subsystem check |
| `drivers/virtio/{mod,transport,modern,legacy,pci,virtqueue}.rs` | new; `legacy_layout`/ring code copied from `virtio_blk.rs`, which **stays untouched** until M4 |
| `drivers/mod.rs` | module wiring, deprecated `legacy_io` re-export |
| `src/core/irq_vectors.asm`, `src/core/idt.c`, `src/core/irq.c` | stubs + catch-all fill (0x80 preserved) + `vector_dispatch` hook |
| `arch/x86/{mod,msr,lapic,vectors}.rs` | LAPIC init (no `sti` change on UEFI yet), self-IPI test |
| `src/core/shell.c`, `src/includes/rust_abi.h` | `irq`, `pci -caps` |
| `scripts/lib/lint.sh`, `scripts/core.sh` | grant lint verb |
| `src/rust/popcorn_logic_tests/` (+ workspace `Cargo.toml`) | host test crate (§9.2) |

**Exit:**
- `pci -caps` shows the virtio vendor caps and MSI-X on virtio-blk, NVMe and xHCI.
- On GRUB, `irq` shows the self-IPI counted once.
- In a dedicated test run, ModernTransport reads virtio-blk's capacity (with `disable-modern=off`) and it matches the legacy driver. This proves the modern transport before net uses it.
- Host SplitQueue tests are green.

### M1: polled virtio-net TX/RX + ARP + ICMP ping

| Files | Change |
|---|---|
| `drivers/virtio/net.rs`, `drivers/net/{mod,loopback}.rs` | driver, trait, loopback |
| `drivers/device.rs`, `drivers/registry.rs` | `DeviceClass::Net`, `net0` catalog entry |
| `src/net/{mod,iface,pump,checksum,eth,arp,ipv4,icmp,cmd}.rs` | stack core |
| `src/core/kernel.c` | `rust_io_pump()` after `timer_poll()` |
| `src/core/shell.c`, `src/includes/rust_abi.h` | `net`, `ping` |
| `scripts/lib/qemu-uefi.sh`, `scripts/lib/kernel.sh`, `scripts/popcorn_build/qemu.py` | `qemu_uefi_net_args` (with `vectors=0`), mirrored on GRUB and in the GUI runner |
| `scripts/lib/qemu-uefi.sh`, `scripts/core.sh`, `src/rust/popcorn_kernel/Cargo.toml` | `qemu_uefi_test_net`, `test-net` verb, `net-selftest` feature + `POPCORN_TEST_NET` cflag (unflashable image guard copied from `qemu-install-selftest`) |
| `scripts/lib/pcapcheck.py` | stdlib pcap checker |

**Exit:** the serial log shows `net: probe ok`, `net: arp 10.0.2.2 52550a000202`, `net: ping 4/4` on UEFI and GRUB. The pcap is clean. A no-NIC boot logs `net: none`.

### M2: LAPIC + MSI-X hybrid

| Files | Change |
|---|---|
| `drivers/bus/msix.rs`, `drivers/grant/irq.rs` | `alloc_irqs` |
| `arch/x86/lapic.rs` | LINT programming per path |
| `src/core/kernel.c` | UEFI `sti` behind Q1 with PICs + LINT0 masked; `cli; check; sti; hlt` idle |
| `drivers/virtio/{net,virtqueue}.rs` | vectors, arm/disarm, EVENT_IDX accepted |
| `scripts/lib/qemu-uefi.sh` | default vectors; matrix flags |

**Exit:**
- With vectors on: `net: irq msix v=0x4N`, `irqs > 0`, ping 4/4.
- `vectors=0` still passes in poll mode.
- In poll mode, MSI deliveries = 0.

### M3: UDP / DHCP / DNS

| Files | Change |
|---|---|
| `src/net/{udp,dhcp,dns}.rs`, `src/net/cmd.rs` | protocols, commands |
| `src/core/shell.c` | `udp`, `nslookup`, `net -dhcp` |
| `scripts/lib/qemu-uefi.sh` | `hostfwd` + `test-net` DHCP/DNS/UDP assertions |
| CSUM/GUEST_CSUM | accepted only when offered (tap run) |

**Exit:**
- `net: dhcp 10.0.2.15 gw 10.0.2.2 dns 10.0.2.3`.
- `nslookup` resolves a name when the host has DNS. The CI assertion is "query sent + response or timeout logged", because CI may have no external DNS.
- The host↔guest UDP echo over hostfwd passes.

### M4: virtio-blk migrated

| Files | Change |
|---|---|
| `drivers/virtio/blk.rs` | new |
| `drivers/block/virtio_blk.rs` | becomes `pub use crate::drivers::virtio::blk::{probe_all, capacity, read, write, SECTOR_SIZE, MAX_VIRTIO_DISKS};` |
| `scripts/lib/qemu-uefi.sh` | run blk both `disable-modern=on` and `off` |

The migration in detail:
- `Queue` impl with requests in flight up to `size/3`.
- `DmaArray<BlkReqHdr>` + status bytes; a 4 KiB bounce `BufSlab`.
- A dynamic config offset.
- A tick-based timeout instead of 10M spins; on timeout, reset and poison.
- MSI-X with one request vector + config.
- Accept SEG_MAX(2), RO(5) (honour it), BLK_SIZE(6), FLUSH(9) (issued after `install`/`wipe`), VERSION_1. Refuse GEOMETRY(4), TOPOLOGY(10), CONFIG_WCE(11), MQ(12), DISCARD(13), WRITE_ZEROES(14).

**Exit:**
- `disk -list/-info/-read/-write`, the FAT32 tests and `rust_test_disk_install` pass on both transports.
- Queue depth > 1.
- The lint finds no raw I/O under `drivers/virtio/`.

Later work: NVMe and xHCI onto `Queue` + grants; packed ring and IN_ORDER experiments; tap offload; `opt-level` review.

---

## 9. Test plan

### 9.1 Log tags

Tags are full lowercase lines on **COM1 only**, prefixed `net: ` and written through `com1_puts`. Debugcon is reserved for the existing single-character milestones (F8). Examples: `net: none`, `net: probe ok`, `net: probe fail <why>`, `net: mac 525400123456`, `net: feat dev=… acked=…`, `net: link up|down`, `net: irq poll|msix v=0x41`, `net: arp 10.0.2.2 52550a000202`, `net: ping 4/4`, `net: dhcp …`, `net: csum ok`, `net: done`.

### 9.2 Host unit tests (`src/rust/popcorn_logic_tests`, std, `#[path]` includes)

- **SplitQueue against a software device model.** The model consumes avail and produces used in **random order** with random lengths, and starts `used.idx` near 65530. It covers:
  - u16 wrap;
  - `vring_need_event` edges (event == old, event == new−1, distance 0x8000);
  - arm races, where the model completes between the `used_event` write and the re-check;
  - a full ring;
  - the free list never handing out a descriptor twice, run as a 1M-op property test;
  - both EVENT_IDX on and off.
- `checksum`: compared against a byte-wise reference on lengths 0–2000 at odd alignments, plus RFC 1071 vectors.
- `eth`/`arp`/`ipv4`/`icmp`/`udp`/`dhcp`/`dns` parse and build round-trips against captured packets, plus a no-panic sweep over 1M random and mutated frames.
- `BufSlab` address math and the chunked pool.

### 9.3 In-kernel

- `net -selftest` over `loopback.rs`: ARP, ping self, UDP round-trip, DHCP against an in-kernel fake server, pool exhaustion.
- The LAPIC self-IPI on every pool vector, counted exactly once.
- `ALLOC_COUNT: AtomicU64` in `alloc_shim.rs`. A 100k-frame TX burst and an RX flood must show **0** heap allocations during the run, as a hard pass/fail.

### 9.4 Automated QEMU (`./scripts/core.sh test-net`)

- Build with `-DPOPCORN_TEST_NET` + `net-selftest`.
- The kernel runs `rust_net_selftest()` after `phase2_selftest()`: up, ARP, 20 pings, (M3) DHCP + DNS + UDP, then `net: done`.
- The script waits up to 60 s on the **serial** log.
- The pcap checker (stdlib `struct`) on `net.pcap` verifies:
  - the ARP request from our MAC and the reply from `52:55:0a:00:02:02`;
  - echo id/seq pairs;
  - our IPv4/ICMP/UDP checksums, recomputed. This catches endianness bugs that slirp drops silently.

### 9.5 Matrix

| Case | Flags | Expect |
|---|---|---|
| UEFI q35 poll | `vectors=0` | `irq poll`, ping 20/20 |
| UEFI q35 MSI-X (M2, Q1) | default | `irq msix`, irqs > 0 |
| GRUB pc, poll and MSI-X | both | same; PIC IRQ0/1 keep working |
| EVENT_IDX off | `-global virtio-net-pci.event_idx=off` | NO_NOTIFY / NO_INTERRUPT path passes |
| Transitional device | `disable-legacy=off` (id 0x1000 + caps) | modern chosen; legacy override also works (if Q2 = yes) |
| No NIC | net args omitted | `net: none`; existing tests green |
| Default NIC | no `-netdev` | PCI print shows e1000e/e1000; driver ignores it |
| tap offload (M3, optional) | `-netdev tap,vnet_hdr=on` | CSUM/GUEST_CSUM acked; pcap checksums valid |
| Concurrency | virtio-blk + NVMe + USB-MSC attached, `disk` I/O during ping | no vector or lock conflicts |

### 9.6 Stress and fault tests

- Host blasts 64-byte UDP at hostfwd :5555 for 10 s. The heartbeat and shell stay responsive. Afterwards `rx_posted == RX_SLOTS` and ping passes.
- `ping 10.0.2.2 100 1472` (maximum frame both ways; `tx_full` may increase, no hang).
- `ping 10.0.2.99 3`: three ~1 s timeouts, ARP retries visible in the pcap.
- `set_link pnet0 off/on` over the HMP socket, 100×: the config vector or poll toggles `link_up`, and recovery works.
- `net -down; net -up` × 50: `outstanding_pages == 0`, meminfo stable.
- 10 000 pings plus the flood, enough to push more than 65 536 RX descriptors through (wrap).
- Malformed frames (truncated, bad IHL, bad checksum) injected through a `-netdev socket` run: counted, never a panic.
- 30-minute soak: disk I/O + UDP echo.
- Regression after every milestone: `disk` tests, install selftest, boot stability.

---

## 10. QEMU flags (`qemu_uefi_net_args`, selected by `POPCORN_QEMU_NET=user|tap|off`)

```
-netdev user,id=pnet0,ipv6=off,hostfwd=udp:127.0.0.1:5555-:5555
-device virtio-net-pci,netdev=pnet0,id=pnic0,mac=52:54:00:12:34:56,disable-legacy=on,romfile=,vectors=${POPCORN_NET_VECTORS:-0}
-object filter-dump,id=pdump0,netdev=pnet0,file=$POPCORN_TARGET/net.pcap      # test runs
# tap (Linux/WSL2 only; the only backend that exposes CSUM/GUEST_CSUM):
-netdev tap,id=pnet0,ifname=poptap0,script=no,downscript=no,vnet_hdr=on
```

- **`disable-legacy=on`** gives device id 0x1041, no I/O BAR, and the modern transport.
- **`romfile=`** stops the iPXE option ROM.
- **OVMF's built-in VirtioNetDxe** can still add PXE/HTTP boot entries, so add `bootindex=0` to the existing `usb-storage,drive=usbstick` device (this changes the shared boot device line; Q11).
- **`vectors=0`** hides MSI-X and forces poll mode in M1. From M2 the default is unset; QEMU's default is `2·queue_pairs + 2` = 4, and older QEMU used 3. Tests run both.
- **An explicit `-netdev`** suppresses the default e1000e/e1000 (F6).
- **Mirror the same pair** in `run_legacy_qemu` / `qemu_legacy_smoke` (`kernel.sh:275,288`, pc machine), so the GRUB IRQ path is tested too, and in `scripts/popcorn_build/qemu.py`.
- **Host bindings** stay on 127.0.0.1, consistent with the scripts' "host disks never attached" stance.
- **Debug:** `-trace virtio_queue_notify -trace virtio_notify` checks kick batching.

---

## 11. Risks

1. **The UEFI interrupt hang may survive the catch-all fill.** Mitigation: UEFI stays polled and MSI-X is proven on GRUB first. Decision 3 explicitly allows this.
2. **Modern transport is unproven in-tree.** Mitigation: the M0 test reads blk capacity through ModernTransport before net depends on it.
3. **Low MMIO cacheability.** WB direct-map aliases of MMIO depend on MTRRs. `map_mmio_uc` covers our accesses; removing the aliases is Q8.
4. **Firmware leaves the NIC live** (OVMF VirtioNetDxe) or PXE delays boot. Mitigation: reset before bus mastering, `romfile=`, `bootindex=0`.
5. **Default NIC already present** (F6). Mitigation: the explicit `-netdev`; the driver must ignore e1000(e).
6. **Debugcon grep collisions** (F8). Mitigation: COM1-only tags. The longer-term fix is anchored grep patterns.
7. **Allocator pressure** (F1). Mitigation: chunked pools; the zero-alloc gate catches regressions. A real slab allocator is Q9.
8. **Header 10/12 and config 20/24 bugs** on the legacy path. Mitigation: negotiated fields, a MAC re-read check, pcap validation.
9. **slirp limits:** no checksum offload, and external ICMP depends on host permission. Only 10.0.2.2 is in CI scope.
10. **Spec drift.** Constants here were re-checked against VIRTIO 1.2/1.3 (common cfg, cap layout, net bits incl. 1.3 additions 52–56, legacy MSI-X offsets, the 1526 B rule, the DRIVER_OK notify rule) and the Intel SDM for the LAPIC/MSI formats. Re-verify anything new against the OASIS text while coding.

---

## 12. Roadmap alignment (for the doc-fix pass, not done here)

- `roadmap.md:34` says "PCI walk Done (bus 0)". `pci.rs` already recurses through bridges.
- `roadmap.md:146` "DMA helper (contiguous, below 4 GiB) — stub exists" is stale. `dma_alloc` (`dma.rs:90`) passes `MEM_ALLOC_DMA`. `kmalloc` maps that to `ZONE_DMA`, and `zone_alloc` limits `pmm_alloc_contig` to frames below 4 GiB (`memory.c:181,328,444`). Tick the item and point it at `dma_alloc`/`map_mmio`. The missing UC mapping below 64 GiB (F3) is a separate open item.
- `roadmap.md:201` "IOAPIC later": add "LAPIC + generic vector stubs (0x30–0xFF)".
- `roadmap.md:310–311` (ACPI before IOAPIC): under decision 3, PCI devices use MSI-X straight to the LAPIC, so **neither ACPI nor the IOAPIC blocks networking**. Reword to "LAPIC + MSI-X for PCI now; ACPI/IOAPIC later for ISA routing/HPET".
- `roadmap.md:13, 313, 325` gate the NIC behind block and Phase 5. Replace with M0–M4. virtio-blk migration (M4) becomes the block-stable gate.
- `roadmap-UTD.md`:
  - **Phase 21:** LAPIC basics and "MSI/MSI-X: Later" are pulled forward. ACPI/IOAPIC are not.
  - **Phase 3.2 (common ASM wrapper):** mostly delivered by the current `idt.asm`: `PUSH_GPRS`/`POP_GPRS` order, fake error code + vector push (`EXC_NOERR`/`EXC_ERR`), `cld`, 16-byte alignment before C calls, `fxsave64`, exact restore + `iretq`, and the frame layout pinned by `_Static_assert` in `exception.c`. Tick those items. `irq_vectors.asm` extends the same wrapper to 0x30–0xFF. Still open: the `*PopFrame` type passed to C.
  - **Phases 22–23 (`*PopIRQ`, MMIO/IO caps):** the grants are the in-kernel precursors.
  - **Phase 26.1:** "first QEMU virtio-net" is this work.
  - **Phases 0.2/26.2:** `src/net/` is the bootstrap for `*PopNet`.

---

## 13. Open questions for the owner

1. **(Settled 23 Nov: UEFI has interrupts via the LAPIC virtual wire.)** **UEFI interrupts:** in M2, turn IF on with both PICs and LINT0 masked and only LAPIC/MSI vectors live, after the catch-all fill? Or keep UEFI poll-only for now?
2. **(Answered 23 Nov: modern-only for net.)** **Legacy virtio-net:** support the transitional/legacy path for net (10 B header, ANY_LAYOUT split, 20/24 shift; about 100 lines on top of the shared legacy transport)? Or modern-only for net, with legacy kept just for blk until M4? The proposal is modern-only.
3. **(Answered: probe at boot, configure on `net -up`.)** **Boot behaviour:** probe at boot and configure on `net -up` (proposed), or auto-DHCP at boot?
4. **(Answered: kmain loop for now; an `io` task later.)** **Where the pump runs:** in the kmain loop (proposed, matches today), or in a dedicated `io` kernel task in `scheduler.c` woken by `IrqHandle`? The task is the ring-3 shape.
5. **(Settled: C `lapic.c`, exposed to Rust.)** **LAPIC location:** Rust `arch/x86/` (proposed, next to the grant layer) or C `src/core/apic.c` next to the PIC code?
6. **(Answered: after M4.)** **Perf experiments:** schedule packed ring / IN_ORDER / tap checksum offload after M4, or drop them until TCP?
7. **(Answered: accepted.)** **Memory budget:** about 1 MiB DMA per NIC (256 + 256 × 2 KiB) and about 128 KiB per virtio disk. Acceptable?
8. **(Answered: separate `map_mmio_uc`, no page splitting.)** **MMIO cacheability:** may M0 split the 2 MiB direct-map pages over PCI/LAPIC holes and mark them UC (removing WB aliases), beyond adding `map_mmio_uc`?
9. **(Answered: inside M0 — done.)** **Cleanup scope:** convert the existing ~67 `static mut` to `IrqSpinLock`/`Once`, and add a small slab allocator in front of page-granular `kmalloc`, in this effort or as a separate pass?
10. **(Answered: when the first MSI-only device arrives — that is the target laptop's Qualcomm WCN6855 Wi-Fi, PCI 17CB:1103, which uses plain MSI.)** **Plain MSI fallback:** implement now (unused by virtio), or when the first MSI-only device (e.g. e1000) arrives?
11. **(Answered: yes.)** **Boot order:** OK to add `bootindex=0` to the USB stick device in all QEMU runs?
12. **(Answered: static fallback after 3 s.)** **DHCP failure:** fall back to static 10.0.2.15/24 after 3 s (proposed), or stay unconfigured?
13. **(Answered: deprecation warnings + lint — done.)** **Grant enforcement strength:** deprecation warnings + lint now (proposed), or hard visibility that forces NVMe/xHCI/kbd/serial onto grants in the same release?
14. **(Answered: descriptive names.)** **Naming:** keep descriptive names (`MmioRegion`, `IrqHandle`, `DmaPool`, `net0`, `drivers/net` + `src/net`), or adopt the UTD names (`*PopIO`, `*PopIRQ`, `*PopMem`, `*PopNet`) now?
15. **(Answered: yes; new `net:` tags are COM1-only from M1.)** **Diagnostics channel:** move all multi-character serial output (including `scheduler.c` `serial_print`) to COM1-only and anchor the harness greps, as part of the repo cleanup?
