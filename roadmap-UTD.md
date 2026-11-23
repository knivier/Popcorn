# Popcorn — ExoCore Master Build Checklist

## Legend

- `[x]` — already implemented or substantially present
- `[ ]` — not finished / must verify
- `*Name` — **Popcorn-specific design or proposed custom name**
- Unstarred names such as `GDT`, `TSS`, `ELF`, `APIC`, `PCIe` are standard mechanisms
- **GATE** — do not seriously build the next layer until this works
- **OPTIONAL** — not necessary for the initial ExoCore

---

# Architectural North Star

Popcorn is **not trying to become Linux**.

The intended hierarchy is:

```text
Applications
│
├── *LibPop applications
├── *PopNative applications
├── compatibility personalities
└── specialized exo applications
        │
        ▼
   *PopABI / *ExoABI
        │
        ▼
┌──────────────────────────────┐
│       *PopCore / ExoCore     │
│                              │
│ Protection                   │
│ Capabilities                 │
│ CPU scheduling mechanism     │
│ Memory ownership             │
│ IRQ/resource ownership       │
│ IPC/event primitives         │
│ Address spaces               │
│ Resource revocation          │
└──────────────┬───────────────┘
               │
               ▼
            Hardware
```

The kernel should increasingly answer:

> “Who owns this resource, and are they allowed to perform this operation?”

instead of:

> “What filesystem policy, allocator policy, network policy, process policy, and device policy should everyone use?”

Higher-level policy belongs in libraries, services, and personalities.

---

# PHASE 0 — Freeze the Ground Truth

## 0.1 Create one architecture document

- [ ] Create `docs/architecture.md`
- [ ] State that Popcorn is a hybrid exokernel/monolithic OS
- [ ] Define what remains permanently Ring 0
- [ ] Define what should eventually be movable to Ring 3
- [ ] Define what “monolithic compatibility layer” means
- [ ] Define what “ExoCore” means specifically for Popcorn
- [ ] Explicitly state that POSIX compatibility is optional
- [ ] Explicitly state that Linux ABI compatibility is not a design requirement
- [ ] Define the kernel's minimum trusted computing base
- [ ] Add a diagram of Ring 0 vs Ring 3 ownership

## 0.2 Pick placeholder Popcorn names

Suggested names:

ABI's are pipelines for ring3 -> ring 0 access

- [ ] `*Core` — minimal privileged ExoCore
- [ ] `*SecurePipe` — common syscall/application ABI
- [ ] `*DirectPipe` — raw low-level resource ABI
- [ ] `*UserPipe` — default userspace OS/library personality
- [ ] `*PopRealm` — protection/resource domain
- [ ] `*PopTask` — schedulable execution object
- [ ] `*PopSpace` — address-space resource
- [ ] `*PopCap` — capability handle
- [ ] `*PopGrant` — delegated capability
- [ ] `*PopEvent` — kernel event object
- [ ] `*PopPort` — IPC endpoint
- [ ] `*PopMem` — memory resource
- [ ] `*PopIRQ` — interrupt resource
- [ ] `*PopIO` — port/MMIO resource
- [ ] `*PopBlock` — block-device extent/resource
- [ ] `*PopDevice` — compatibility-layer device abstraction
- [ ] `*PopFS` — default userspace filesystem personality
- [ ] `*PopNet` — default userspace networking personality
- [ ] `*PopGraph` — graphics/windowing service
- [ ] `*PopPkg` — package/application format
- [ ] `*PopManifest` — application/resource manifest

These are placeholders. Rename freely.

---

# PHASE 1 — Repair Current Hardware Reliability

# 1.1 Real-hardware block write debugging

Do this before expanding storage.

- [ ] Select a disposable USB/storage target
- [ ] Confirm correct physical device is selected
- [x] Print selected device identifier
- [ ] Print block size
- [x] Print total block count
- [x] Print whether device is writable
- [x] Print whether write lock is active
- [ ] Print requested LBA before every test write
- [ ] Print requested sector count
- [ ] Print buffer physical address
- [ ] Print buffer virtual address
- [ ] Print hardware command result
- [ ] Print controller status after completion
- [ ] Print timeout reason when command fails

### Raw write test

- [ ] Choose one sacrificial sector
- [ ] Read sector
- [ ] Save original bytes
- [ ] Write known pattern
- [ ] Read same sector immediately
- [ ] Compare every byte
- [ ] Flush device if supported
- [ ] Reset/reinitialize controller
- [ ] Read sector again
- [ ] Reboot machine
- [ ] Read sector again
- [ ] Restore original bytes

### Layer isolation

- [x] Verify `ram0` write path (QEMU: `disk_ram0_raw_sector_io`)
- [x] Verify virtio-blk write path (QEMU: FAT32 files on `vda`, `ported_disk_switch_fat_isolation`)
- [ ] Verify QEMU NVMe write path
- [ ] Verify real NVMe write path
- [ ] Verify real USB MSC write path
- [x] Verify FAT32 only after raw block path works (QEMU, `test-shell`; real hardware still open)
- [x] Test FAT directory update (QEMU, `test-shell`; real hardware still open)
- [x] Test cluster allocation (QEMU: `fs_large_multicluster_file_*`; every scratch disk is checked with `fsck.fat -n`)
- [ ] Test file extension
- [x] Test file overwrite (QEMU, `test-shell`; real hardware still open)
- [x] Test file truncate (QEMU, `test-shell`; real hardware still open)
- [x] Test file persistence after reboot (QEMU: `persistence_across_reboot`, `ported_reboot_repeat_persistence`)

### GATE 1

- [ ] Real hardware can write one raw sector and read identical data after reboot
- [ ] FAT32 can persist one newly created file

---

# PHASE 2 — Correct the Interrupt Foundation

## 2.1 PIC cleanup

- [x] Change PIC master ICW3 to `0x04`
- [x] Change PIC slave ICW3 to `0x02`
- [x] Add `io_wait()` between PIC initialization commands
- [x] Preserve existing IRQ masks where appropriate
- [x] Ensure IRQ0 can be independently masked/unmasked
- [x] Ensure IRQ1 can be independently masked/unmasked
- [x] When enabling IRQ8–15, automatically unmask master IRQ2
- [x] Add helper `pic_send_eoi(irq)`
- [x] Correct slave-before-master EOI ordering
- [x] Add spurious IRQ7 detection
- [x] Add spurious IRQ15 detection
- [x] Record unexpected IRQ count

DONE!

## 2.2 PIT correction

- [x] Define exactly what one `global_timer.tick` means (one PIT period = `pit_divisor` input clocks, PIT mode 2; `src/core/timer.c`)
- [x] Keep that meaning identical in IRQ and polling mode
- [x] Fix polling code so PIT decrements are not treated as timer periods
- [ ] Verify 100 Hz means approximately 100 ticks per real second
- [ ] Measure 10-second timer interval
- [ ] Compare timer result with an external clock
- [x] Test GRUB PIT IRQ mode
- [x] Test UEFI timer: UEFI now runs the PIT on IRQ0 too (LAPIC virtual wire, `src/core/lapic.c`); polling is only the fallback when `timer_irq_check()` sees no IRQ0
- [ ] Verify both report similar uptime
- [ ] Verify sleep(1000 ms) lasts approximately one second
- [ ] Define constant version number to pull from as popcorn constant vcn
- [x] Fetch RTC data using Port 0x70 / 0x71

## 2.3 Build flags

- [x] Verify all interruptible x86-64 kernel C code uses `-mno-red-zone`
- [x] Verify freestanding compiler flags
- [x] Verify stack protector policy
- [x] Verify no userspace libc assumptions leak into kernel build
- [ ] Document required ABI flags

---

# PHASE 3 — Standardize Interrupt Entry

## 3.1 Define one interrupt frame

Create a standard structure such as:

```c
typedef struct {
    uint64_t r15;
    uint64_t r14;
    uint64_t r13;
    uint64_t r12;
    uint64_t r11;
    uint64_t r10;
    uint64_t r9;
    uint64_t r8;

    uint64_t rbp;
    uint64_t rdi;
    uint64_t rsi;
    uint64_t rdx;
    uint64_t rcx;
    uint64_t rbx;
    uint64_t rax;

    uint64_t vector;
    uint64_t error;

    uint64_t rip;
    uint64_t cs;
    uint64_t rflags;

    /* present when privilege changes */
    uint64_t rsp;
    uint64_t ss;
} *InterruptFrame;
```

Name suggestion:

- [x] `*PopFrame` (`src/includes/popframe.h`: GPRs, vector, error, CPU frame; FXSAVE area just below it)

## 3.2 Common ASM interrupt wrapper

- [x] Save all GPRs in one consistent order
- [x] Normalize exception error-code layout
- [x] Push fake error code for exceptions without CPU error code
- [x] Push vector number (exceptions push the vector; IRQ stubs push the ISA IRQ number)
- [x] Execute `cld`
- [x] Align stack correctly before C calls
- [x] Pass `*PopFrame*` to C (`isr_common` in `idt.asm` → `isr_dispatch(PopFrame*)` in `src/core/isr.c`, for all 256 vectors)
- [x] Restore original stack exactly (IRQ and int 0x80 paths; exceptions halt)
- [x] Restore GPRs
- [x] Remove normalized vector/error fields (`add rsp, 16` before `iretq`)
- [x] Finish with `iretq`
- [x] Verify no C code sees a malformed stack (`_Static_assert` layout checks + runtime 16-byte alignment check in `isr_dispatch`)

## 3.3 Exceptions

Add dedicated handlers for:

- [ ] \#DE divide error
- [ ] \#DB debug
- [ ] \#BP breakpoint
- [ ] \#OF overflow
- [ ] \#BR bound-range
- [ ] \#UD invalid opcode
- [ ] \#NM device unavailable
- [ ] \#DF double fault
- [ ] \#TS invalid TSS
- [ ] \#NP segment not present
- [ ] \#SS stack fault
- [ ] \#GP general protection
- [ ] \#PF page fault
- [ ] \#MF x87 FP
- [ ] \#AC alignment check
- [ ] \#MC machine check
- [ ] \#XM SIMD FP

## 3.4 Fault reporting

- [x] Print vector
- [x] Print error code
- [x] Print RIP
- [x] Print RSP
- [x] Print CS
- [x] Print SS where available
- [x] Print RFLAGS
- [x] Print CR2 for #PF
- [x] Decode #PF bits
- [x] Print CR3
- [x] Print current `*PopTask` (pid)
- [ ] Print current `*PopRealm`
- [x] Dump a few stack words (only when RSP is in mapped kernel memory)
- [x] Halt only for fatal Ring-0 faults (`#DB` / `#BP` are reported and resumed)

### GATE 2

- [ ] Exception self-tests work
- [x] Timer IRQ runs continuously (both boot paths; 60 s preemption soak in `test-shell`)
- [x] Keyboard IRQ runs continuously (IRQ1 on both boot paths; PS/2 typing cases)
- [x] No random crashes from ABI stack alignment

---

# PHASE 4 — Rebuild Context Switching Properly

## 4.1 Separate concepts

Stop treating all of these as one object:

- [ ] Define `*PopTask` = CPU execution context
- [ ] Define `*PopRealm` = protection/resource ownership domain
- [ ] Define `*PopSpace` = address space
- [ ] Allow one `*PopRealm` to eventually own multiple `*PopTask`s
- [ ] Allow multiple tasks to share one address space later

## 4.2 Switch using interrupt frames

> Done (23 Nov 2026): every switch is a frame switch. A task that is not running is one saved `*PopFrame` on its kernel stack (`TaskStruct.frame`). `isr_dispatch` returns the frame to resume; `switch_frames()` in `scheduler.c` stores the outgoing frame and returns the incoming one, and `isr_common` restores FXSAVE + GPRs and `iretq`s into it. Voluntary switches use `int 0x81`, so yield, block, sleep and exit take the same path; new tasks start from a prepared frame. `switch_to()` is gone.

- [x] Timer interrupt receives `*PopFrame`
- [x] Save interrupted frame into current task (`TaskStruct.frame`)
- [x] Scheduler chooses next task (`pick_next`)
- [x] Load selected task's frame
- [x] Restore CR3 when address space differs (`task_switch`)
- [x] Return through `iretq` (preempted task resumes via `irq_common`)
- [x] Remove timer-preemption dependency on normal C `context_save()`
- [x] Keep cooperative yield behavior compatible (`scheduler_yield` uses the same `schedule_locked` path)

## 4.3 Cooperative scheduling

- [x] Implement `yield`
- [x] Make `yield` enter same scheduler mechanism
- [ ] Verify yielding task resumes exactly after yield
- [ ] Verify callee-saved registers survive
- [ ] Verify caller-saved registers behave according to ABI
- [ ] Verify stack pointer survives

## 4.4 Kernel task trampoline

Suggested custom name:

- [x] `*PopTaskEntry` (implemented as `task_entry_trampoline` in context_switch.asm + `scheduler_task_entry`; exit path is `scheduler_exit_current`)

Flow:

```text
iretq/context restore
       ↓
*PopTaskEntry
       ↓
task_function(argument)
       ↓
*PopTaskExit
```

Tasks:

- [x] Ensure C entry stack obeys SysV ABI
- [x] Provide task argument (`scheduler_task_entry` calls the task function with its data in RDI)
- [x] Catch normal task return
- [x] Mark returned task dead
- [x] Reschedule
- [x] Never fall off stack into garbage

## 4.5 Scheduler stress tests

- [x] Create task A counter (`mon -stress`, `src/core/stress.c`)
- [x] Create task B counter
- [x] Create task C counter
- [x] Preempt all three at 100 Hz (they never yield; 5-tick quantum)
- [x] Run 1 minute (`preemption_stress_soak_60s`, both boot paths, ~1,500 switches, 0 errors)
- [ ] Run 10 minutes
- [ ] Run 1 hour
- [x] Verify all counters progress
- [x] Verify no register corruption (rbx, r8-r15, xmm0/xmm1 checked across every spin)
- [ ] Verify no stack corruption
- [x] Verify keyboard remains responsive (the shell answers while the spinners run)
- [x] Verify filesystem still works (wipe/write/read under load)
- [x] Verify sleep/wake works (sleeper task in `mon -stress`)

### GATE 3

- [ ] Millions of context switches without corruption

---

# PHASE 5 — Clean the GDT/TSS Design

## 5.1 Standard selector map

Eliminate magic numbers.

Suggested layout:

```text
0x00  null
0x08  kernel code
0x10  kernel data
0x18  user data
0x20  user code
0x28  TSS low
0x30  TSS high
```

- [ ] Define selectors in one header
- [ ] Replace literal `0x08`
- [ ] Replace literal `0x10`
- [ ] Define user selectors with RPL3
- [ ] Define DPL3 user code descriptor
- [ ] Define DPL3 user data descriptor
- [x] Reload segment registers appropriately
- [ ] Verify CS after GDT initialization
- [ ] Verify TR after `ltr`

## 5.2 TSS

- [ ] Create one clearly typed x86-64 TSS struct
- [ ] Stop manually writing magic byte offsets where possible
- [x] Set `rsp0` (static boot `stack_top` only; per-task RSP0 update still open)
- [x] Keep dedicated #DF IST (IST2, 4 KiB)
- [x] Keep dedicated critical-fault IST (IST1, 4 KiB, used by #PF and #GP)
- [ ] Add guard pages to IST stacks later
- [ ] Document IST numbering
- [ ] Verify `ltr` succeeds
- [ ] Verify TSS descriptor layout

---

# PHASE 6 — Build the Ring-3 Memory Boundary

## 6.1 User mappings

- [ ] Add `vmm_map_user_4k()`
- [ ] Add `vmm_unmap_user_4k()`
- [ ] Add `vmm_map_user_range()`
- [ ] Add read-only user mapping support
- [ ] Add writable user mapping support
- [ ] Add NX user mapping support
- [ ] Ensure U/S bit propagates through PML4
- [ ] Ensure U/S bit propagates through PDPT
- [ ] Ensure U/S bit propagates through PD
- [ ] Ensure U/S bit propagates through PT
- [ ] Ensure kernel mappings remain supervisor-only

Suggested custom wrapper:

- [ ] `*PopSpaceMap()`
- [ ] `*PopSpaceUnmap()`

## 6.2 Define user address layout

Example only:

```text
low/null guard

user executable region
user rodata
user data
user heap

shared runtime region

user stack
guard page

kernel half
```

- [ ] Reserve page zero
- [ ] Pick user image base
- [ ] Pick heap range
- [ ] Pick stack range
- [ ] Add stack guard page
- [ ] Pick shared-library/runtime area
- [ ] Document canonical-address assumptions

## 6.3 Separate user stack and kernel stack

For each `*PopTask`:

- [ ] user stack base
- [ ] user stack top
- [x] kernel stack base (TaskStruct.stack_base; per-task 16 KiB slot from static g_task_stacks pool)
- [x] kernel stack top (TaskStruct.stack_top, 16-byte aligned)
- [x] stack size (TaskStruct.stack_size = TASK_STACK_SIZE)
- [ ] guard page metadata

During scheduling:

- [ ] update TSS `RSP0` to next task's kernel stack
- [ ] assert `RSP0` belongs to current task
- [ ] never use user RSP while executing normal kernel C

### GATE 4

- [ ] Kernel and user pages have visibly different page permissions

---

# PHASE 7 — First Ring-3 Program

Do **not** involve ELF yet.

## 7.1 Hard-coded test program

- [ ] Allocate one user code page
- [ ] Allocate one user stack page
- [ ] Write tiny machine-code program into user code page
- [ ] Mark code RX
- [ ] Mark stack RW + NX
- [ ] Prepare user RIP
- [ ] Prepare user RSP
- [ ] Prepare user CS
- [ ] Prepare user SS
- [ ] Prepare RFLAGS
- [ ] `iretq` into CPL3

## 7.2 Verify CPL

- [ ] Read CS in user program
- [ ] Confirm CPL = 3
- [ ] Attempt normal arithmetic
- [ ] Attempt user memory access
- [ ] Attempt privileged instruction
- [ ] Confirm privileged instruction faults

## 7.3 Protection test

- [ ] Attempt reading kernel memory
- [ ] Receive #PF
- [ ] Identify fault as user-originated
- [ ] Kill only offending `*PopTask`
- [ ] Keep kernel alive
- [ ] Keep another task running

### GATE 5 — POPCORN HAS USERSPACE

- [ ] User code executes at CPL3
- [ ] Kernel survives illegal user access

---

# PHASE 8 — Build the Real \*PopABI

## 8.1 Repair `int 0x80`

- [x] Define exact syscall register convention
- [x] Define syscall number register (RAX)
- [x] Define argument registers (RDI, RSI, RDX, RCX, R8, R9; RCX replaces the suggested R10)
- [x] Define return register (RAX)
- [x] Define error convention (negative `SYSCALL_E*`)
- [x] Define preserved registers (all except RAX)
- [x] Define syscall stack frame (`syscall_context_t`)
- [x] Match ASM frame exactly to C struct (`_Static_assert` in syscall.c)
- [x] Preserve return value in RAX
- [ ] Return to user with `iretq`

Suggested ABI:

```text
rax = syscall number
rdi = arg0
rsi = arg1
rdx = arg2
r10 = arg3
r8  = arg4
r9  = arg5

rax = result
```

- [ ] Document as `*PopABI v0`

## 8.2 Minimal Ring-3 syscall set

Initially keep tiny:

- [ ] `debug_write`
- [ ] `yield`
- [ ] `exit`
- [ ] `gettime`

Do **not** expose everything yet.

## 8.3 User pointer handling

Create:

- [ ] `copy_from_user()`
- [ ] `copy_to_user()`
- [ ] `user_range_readable()`
- [ ] `user_range_writable()`
- [ ] `user_string_copy()`
- [ ] maximum copied-string length
- [ ] overflow checking
- [ ] canonical-address validation

Suggested custom names:

- [ ] `*PopCopyIn`
- [ ] `*PopCopyOut`

## 8.4 Ring-3 fault policy

- [ ] Distinguish CPL0/CPL3 faults
- [ ] Kernel fault → kernel panic
- [ ] User fault → terminate task/realm
- [ ] Record reason
- [ ] Record RIP
- [ ] Record CR2
- [ ] Report to parent/debugger later

### GATE 6

- [ ] Ring 3 → syscall → Ring 0 → Ring 3 works repeatedly
- [ ] Invalid user pointer cannot corrupt kernel

---

# PHASE 9 — User Process / Protection Domain Model

Avoid blindly cloning Unix `process`.

Suggested fundamental abstraction:

- [ ] `*PopRealm`

A `*PopRealm` owns:

- [ ] one `*PopSpace`
- [ ] capability table
- [ ] one or more `*PopTask`s
- [ ] accounting state
- [ ] parent/creator relationship if desired
- [ ] exit state
- [ ] resource quota
- [ ] event endpoints

## 9.1 Lifecycle

- [ ] create realm
- [ ] destroy realm
- [ ] create task inside realm
- [ ] destroy task
- [ ] terminate realm when fatal
- [ ] clean address space
- [ ] release capabilities
- [ ] release kernel stacks
- [ ] release user stacks
- [ ] remove scheduler entries

Suggested API:

- [ ] `*realm_create`
- [ ] `*realm_destroy`
- [ ] `*task_spawn`
- [ ] `*task_exit`

## 9.2 Isolation tests

- [ ] Realm A writes its memory
- [ ] Realm B cannot read A private memory
- [ ] Realm B cannot write A private memory
- [ ] Realm A crash does not kill B
- [ ] Realm A cannot access B capabilities
- [ ] CR3 changes correctly
- [ ] TLB behavior is correct

---

# PHASE 10 — ELF64 Loader

## 10.1 ELF parsing

- [ ] Validate ELF magic
- [ ] Validate ELF class = 64-bit
- [ ] Validate little-endian
- [ ] Validate x86-64 machine
- [ ] Validate executable type
- [ ] Validate program-header bounds
- [ ] Iterate PT_LOAD segments
- [ ] Reject malformed offsets
- [ ] Reject integer overflow
- [ ] Reject segments outside user VA

## 10.2 Segment loading

- [ ] Map code RX
- [ ] Map rodata R
- [ ] Map data RW
- [ ] Handle BSS zero fill
- [ ] Respect alignment
- [ ] Set entry point
- [ ] Build initial stack
- [ ] Map guard page

Suggested custom loader:

- [ ] `*PopLoad`

## 10.3 Execute from FAT32

- [ ] Read ELF file through current FAT32 path
- [ ] Load executable
- [ ] Create `*PopRealm`
- [ ] Create `*PopTask`
- [ ] Enter Ring 3
- [ ] Program prints
- [ ] Program exits

### GATE 7

- [ ] Two different ELF binaries run simultaneously

---

# PHASE 11 — User Runtime

## 11.1 Build `*LibPop`

Do not begin with libc.

Start with:

- [ ] raw syscall wrappers
- [ ] `debug_write`
- [ ] `exit`
- [ ] `yield`
- [ ] `gettime`
- [ ] basic memory helpers
- [ ] string length
- [ ] memcpy
- [ ] memset

## 11.2 Define executable entry ABI

- [ ] Define initial stack layout
- [ ] Decide argument passing
- [ ] Decide environment mechanism
- [ ] Decide manifest pointer mechanism
- [ ] Define program entry symbol
- [ ] Add startup assembly
- [ ] Call application `main`
- [ ] call `exit` if main returns

Suggested names:

- [ ] `*PopStart`
- [ ] `*PopMain`

---

# PHASE 12 — Introduce \*PopCaps

This is the start of the actual ExoCore transformation.

## 12.1 Capability identifier

Suggested:

```c
typedef uint64_t PopCap;
```

- [ ] Reserve invalid capability 0
- [ ] Encode handle generation
- [ ] Prevent stale handle reuse
- [ ] Define per-realm capability table

## 12.2 Capability entry

Each entry should track:

- [ ] object type
- [ ] object ID
- [ ] rights
- [ ] owner
- [ ] generation
- [ ] delegation rights
- [ ] revocation state

## 12.3 Rights

Suggested generic rights:

- [ ] READ
- [ ] WRITE
- [ ] MAP
- [ ] EXEC
- [ ] BIND
- [ ] SIGNAL
- [ ] WAIT
- [ ] DUPLICATE
- [ ] GRANT
- [ ] REVOKE
- [ ] CONFIGURE

## 12.4 First capability types

- [ ] `*CAP_MEM`
- [ ] `*CAP_SPACE`
- [ ] `*CAP_TASK`
- [ ] `*CAP_REALM`
- [ ] `*CAP_EVENT`

Later:

- [ ] `*CAP_IRQ`
- [ ] `*CAP_IOPORT`
- [ ] `*CAP_MMIO`
- [ ] `*CAP_BLOCK`
- [ ] `*CAP_DEVICE`
- [ ] `*CAP_CPU`

## 12.5 Capability enforcement

- [ ] Every exo call takes capability handles
- [ ] Validate handle exists
- [ ] Validate generation
- [ ] Validate required rights
- [ ] Validate caller owns/delegated cap
- [ ] Reject unauthorized access
- [ ] Log capability violations

### GATE 8

- [ ] A realm cannot operate on an object without a valid capability

---

# PHASE 13 — \*PopMem: Memory as an Exo Resource

The kernel should manage physical protection.

The userspace allocator should manage allocation policy.

## 13.1 Memory resource object

- [ ] physical frame ownership
- [ ] frame count
- [ ] permissions
- [ ] pinning state
- [ ] DMA suitability
- [ ] mapped address spaces

Suggested:

- [ ] `*PopMem`

## 13.2 Exo memory API

- [ ] `*exo_mem_alloc`
- [ ] `*exo_mem_release`
- [ ] `*exo_mem_map`
- [ ] `*exo_mem_unmap`
- [ ] `*exo_mem_protect`
- [ ] `*exo_mem_grant`

## 13.3 Move malloc policy out of kernel

Current conceptual direction:

```text
malloc
 ↓
*LibPop allocator
 ↓
need more pages
 ↓
*exo_mem_alloc
```

Tasks:

- [ ] implement simple userspace heap allocator
- [ ] stop normal userspace from calling kernel `malloc`
- [ ] kernel only grants pages
- [ ] LibPop manages sub-page allocations

### GATE 9

- [ ] A user process can implement its own allocator entirely above page allocation

---

# PHASE 14 — \*PopEvents

Do not build Unix signals first.

Create a simple event mechanism.

## 14.1 Event object

- [ ] event ID
- [ ] wait queue
- [ ] pending count/state
- [ ] owning realm
- [ ] capability

Suggested:

- [ ] `*PopEvent`

## 14.2 API

- [ ] `*event_create`
- [ ] `*event_wait`
- [ ] `*event_signal`
- [ ] `*event_poll`
- [ ] `*event_destroy`

## 14.3 Integrate scheduler

- [ ] waiting task becomes blocked
- [ ] event signal wakes waiter
- [ ] timeout support
- [ ] multiple waiters
- [ ] event teardown wakes/errors waiters

---

# PHASE 15 — \*PopPorts: IPC

Do not automatically reproduce pipes/sockets/message queues.

Build one Popcorn primitive.

Suggested:

- [ ] `*PopPort`

## 15.1 Port mechanics

- [ ] create port
- [ ] destroy port
- [ ] send small message
- [ ] receive message
- [ ] blocking receive
- [ ] nonblocking receive
- [ ] sender identity
- [ ] queue capacity
- [ ] backpressure

## 15.2 Capability transfer

- [ ] attach capability to message
- [ ] transfer reduced rights
- [ ] duplicate capability
- [ ] deny unauthorized delegation
- [ ] revoke delegated object later

## 15.3 Shared-memory IPC

- [ ] grant memory capability
- [ ] map into second realm
- [ ] establish event pair
- [ ] build zero-copy transport

### GATE 10

- [ ] Two Ring-3 realms communicate without kernel understanding the application protocol

---

# PHASE 16 — \*PopGrants and Resource Delegation

## 16.1 Delegation

- [ ] parent owns object
- [ ] parent can derive reduced-right child capability
- [ ] child cannot increase rights
- [ ] capability inheritance rules defined

Suggested:

- [ ] `*PopGrant`

## 16.2 Revocation

- [ ] revoke one capability
- [ ] revoke child capabilities
- [ ] revoke whole delegation tree
- [ ] remove mappings on memory revoke
- [ ] wake waiters with revoked error
- [ ] safely revoke block resource
- [ ] safely revoke IRQ binding

## 16.3 Quotas

- [ ] maximum memory
- [ ] maximum tasks
- [ ] maximum capabilities
- [ ] maximum IPC ports
- [ ] CPU-time accounting
- [ ] block-resource limits

---

# PHASE 17 — Define the True \*ExoABI

This becomes the low-level OS interface.

## Memory

- [ ] allocate physical resource
- [ ] map memory
- [ ] unmap memory
- [ ] change permissions
- [ ] share memory

## CPU

- [ ] create task
- [ ] destroy task
- [ ] yield
- [ ] control scheduling hints
- [ ] query CPU time

## Events

- [ ] create event
- [ ] wait
- [ ] signal

## IPC

- [ ] create port
- [ ] send
- [ ] receive

## Resources

- [ ] query capability
- [ ] grant capability
- [ ] revoke capability
- [ ] inspect owned resources

## Hardware

Later:

- [ ] bind IRQ
- [ ] map MMIO
- [ ] grant I/O port
- [ ] access block extent

### GATE 11 — BASIC EXOCORE EXISTS

- [ ] Kernel primarily exposes low-level resources
- [ ] Resource ownership is capability enforced
- [ ] Higher-level policy can be implemented in Ring 3

---

# PHASE 18 — Turn Current Monolithic APIs into a Personality

Do not throw your existing work away.

Create:

- [ ] `*PopCompat`

`*PopCompat` initially contains your familiar APIs:

- [ ] open
- [ ] read
- [ ] write
- [ ] close
- [ ] ioctl
- [ ] filesystem naming
- [ ] convenience sleep
- [ ] compatibility process APIs

Initially some may still call kernel services.

Gradually move them into `*LibPop`.

The target becomes:

```text
application
    ↓
*PopCompat
    ↓
*LibPop
    ↓
*ExoABI
```

instead of:

```text
application
    ↓
huge kernel syscall API
```

---

# PHASE 19 — Move FD Policy into \*LibPop

## 19.1 Userspace FD table

- [ ] create userspace fd structure
- [ ] reserve 0/1/2 convention only if desired
- [ ] map fd → LibPop object
- [ ] implement close
- [ ] implement flags
- [ ] implement duplication if desired

## 19.2 Kernel stops caring about fd numbers

Target:

```text
kernel:
    knows capabilities

LibPop:
    knows fd 5
```

- [ ] migrate one test application
- [ ] preserve compatibility API
- [ ] remove kernel fd dependency from exo-native applications

---

# PHASE 20 — Filesystem as Policy

You already have FAT32 code.

Do not discard it.

## 20.1 Define raw block resource

Suggested:

- [ ] `*PopBlock`

Capability contains rights to:

- [ ] read sector range
- [ ] write sector range
- [ ] flush
- [ ] query geometry

## 20.2 First userspace filesystem

Suggested:

- [ ] `*PopFS`

Move gradually:

- [ ] block cache to userspace
- [ ] FAT parser
- [ ] directory traversal
- [ ] file naming
- [ ] allocation policy
- [ ] writeback policy

Kernel eventually only enforces block ownership.

## 20.3 File service option

Applications may use:

```text
app
 ↓
*PopPort
 ↓
*PopFS service
 ↓
*PopBlock
```

or potentially link filesystem logic directly.

Decide which becomes your default architecture.

## 20.4 VFS decision

Do not automatically build Linux VFS.

Choose:

- [ ] no global VFS
- [ ] LibPop VFS
- [ ] filesystem service namespace
- [ ] capability-based object namespace
- [ ] hybrid

Mark final choice in architecture docs.

---

# PHASE 21 — Modern Interrupt Platform

## ACPI

- [ ] locate RSDP
- [ ] validate checksum
- [ ] parse XSDT
- [ ] parse MADT
- [ ] enumerate CPUs
- [ ] enumerate IOAPICs
- [ ] parse interrupt source overrides
- [ ] find HPET table if present
- [ ] parse MCFG for PCIe ECAM later

## Local APIC

- [x] detect APIC support (CPUID + `IA32_APIC_BASE`)
- [x] enable LAPIC (virtual-wire mode: LINT0 = ExtINT, LINT1 = NMI; `src/core/lapic.c`)
- [x] configure spurious vector (0xFF)
- [x] EOI (`lapic_eoi`)
- [ ] timer
- [ ] calibrate timer
- [x] self-IPI delivery test (vector 0x30, counted exactly once at boot; `irq`)

## IOAPIC

- [x] map IOAPIC MMIO (default 0xFEC00000 until MADT parsing; uncached)
- [x] read version (entries are masked so firmware routes cannot fire)
- [ ] program redirection entries
- [ ] route keyboard IRQ
- [ ] route timer where applicable
- [ ] disable legacy PIC during APIC mode

## MSI/MSI-X

Later:

- [x] allocate interrupt vectors (device pool 0x40-0xEF in `drivers/grant/irq.rs`; MSI-X programming is M2)
- [ ] program MSI
- [x] program MSI-X (`drivers/bus/msix.rs`, kernel-owned table)
- [ ] track vector ownership with `*PopIRQ`

---

# PHASE 22 — \*PopIRQ Resource Model

## Kernel

- [ ] represent IRQ/vector as resource
- [ ] associate ownership
- [ ] prevent two incompatible owners
- [ ] mask on owner death
- [ ] support event notification

## Userspace

- [ ] realm receives `*CAP_IRQ`
- [ ] bind IRQ → `*PopEvent`
- [ ] wait on event
- [ ] acknowledge interrupt safely
- [ ] revoke IRQ ownership

Do **not** allow arbitrary interrupt-controller programming from userspace.

---

# PHASE 23 — MMIO and I/O Capabilities

## \*PopIO

- [ ] represent I/O-port range
- [ ] grant read rights
- [ ] grant write rights
- [ ] restrict arbitrary port access

## MMIO

- [x] identify PCI BAR (`drivers/bus/pci.rs` `bar_io` / `bar_mem`, incl. 64-bit sizing; used by virtio-blk, NVMe, xHCI)
- [ ] create MMIO resource
- [ ] create uncached/device mapping
- [ ] map only allowed BAR into realm
- [ ] remove mapping on revocation

## Security

- [ ] prevent mapping arbitrary physical memory
- [ ] prevent mapping kernel RAM
- [ ] prevent conflicting ownership
- [ ] audit resource boundaries

---

# PHASE 24 — Userspace Drivers

Do this selectively.

> **Driver note (owner decisions, 2026-11-22):** near-term drivers stay in Ring 0 as in-kernel, resource-granted Rust drivers. They receive `MmioRegion`/`PortRange`/`IrqHandle`/`DmaPool` grants at probe and never use raw `inl`/`outl`/`phys_to_virt`, so a driver can later move to a Ring-3 realm by swapping the grant implementation. Every device class (block, net, usb, display) uses one submit → kick → complete queue. Interrupts are hybrid: LAPIC + MSI/MSI-X wakes the driver, it polls up to a budget, then re-arms, and pure polling remains the fallback (UEFI today). PCI devices do not use legacy PIC INTx. The network stack is hand-written no_std up to UDP, and TCP will be decided later. No framework code exists yet. Design: `docs/drivers/virtio-net-review.md` ("Locked decisions"). The Ring-3 driver plan below remains the long-term ExoCore target.

## Start easy

Candidates:

- [ ] framebuffer
- [ ] serial after boot
- [ ] virtual/simple devices

Later:

- [ ] PS/2
- [ ] network driver
- [ ] block driver

Do not move boot-critical hardware immediately.

## Driver service design

Suggested:

- [ ] `*PopDrive`

A driver realm receives:

- [ ] MMIO capability
- [ ] IRQ capability
- [ ] DMA capability later
- [ ] IPC port

Driver exposes protocol through `*PopPort`.

---

# PHASE 25 — IOMMU Before Untrusted DMA

For true hardware isolation:

## Detection

- [ ] detect Intel VT-d
- [ ] detect AMD-Vi
- [ ] parse firmware IOMMU tables

## IOMMU domains

- [ ] create DMA address space
- [ ] map owned pages only
- [ ] bind PCI device to domain
- [ ] revoke mappings
- [ ] invalidate IOTLB

Suggested:

- [ ] `*PopDMA`

Without this:

- [ ] mark DMA-capable userspace drivers trusted

---

# PHASE 26 — Networking

Don't copy Linux socket internals unless useful.

## 26.1 Network driver

- [x] first QEMU virtio-net (`drivers/virtio/net.rs`, M1; in-kernel for now, see the driver note in Phase 24)
- [x] TX descriptors
- [x] RX descriptors
- [x] interrupts (M2: MSI-X hybrid — interrupt wakes, poll drains, re-arm; pure polling fallback)
- [x] MAC address
- [x] packet send/receive (ARP + ICMP echo: `ping 10.0.2.2` and `ping 8.8.8.8` work in QEMU)

## 26.2 Userspace network stack

Suggested:

- [ ] `*PopNet`

Implement:

- [ ] Ethernet
- [ ] ARP
- [ ] IPv4
- [ ] ICMP
- [ ] UDP
- [ ] DHCP

Then:

- [ ] TCP
- [ ] DNS

## 26.3 Application API

Design your own object model first.

Possible:

```text
*NetEndpoint
*NetFlow
*DatagramPort
```

Compatibility sockets can be a LibPop wrapper.

---

# PHASE 27 — Graphics

## Framebuffer ownership

- [ ] expose framebuffer capability safely
- [ ] allow graphics service to map framebuffer

Suggested compositor:

- [ ] `*PopGraph`

## \*PopGraph basics

- [ ] framebuffer initialization
- [ ] surface object
- [ ] surface creation
- [ ] surface shared memory
- [ ] basic compositing
- [ ] z-order
- [ ] cursor
- [ ] keyboard events
- [ ] mouse events
- [ ] redraw events

## Window model

Invent your own terminology if desired:

- [ ] `*PopSurface`
- [ ] `*PopView`
- [ ] `*PopScene`

No requirement to copy X11/Wayland.

---

# PHASE 28 — Input System

## Keyboard

- [ ] convert raw scan code to event
- [ ] separate physical key from text input
- [ ] modifier state
- [ ] key repeat

## Mouse

- [ ] PS/2 mouse initially
- [ ] relative movement
- [ ] buttons
- [ ] wheel

## Future

- [ ] USB HID
- [ ] touch input

Suggested userspace protocol:

- [ ] `*PopInput`

---

# PHASE 29 — Time System

Replace PIT dependence.

## Clock sources

- [ ] invariant TSC
- [ ] HPET
- [ ] ACPI PM timer fallback
- [ ] PIT fallback

## Define clocks

Suggested:

- [ ] `*PopMonoTime`
- [ ] `*PopWallTime`

Implement:

- [ ] monotonic nanoseconds
- [ ] wall-clock time
- [ ] deadline timers
- [ ] event-based timers
- [ ] sleep without busy waiting

---

# PHASE 30 — SMP

Do this after single-core architecture is solid.

## CPU discovery

- [ ] MADT CPU enumeration
- [ ] BSP identification
- [ ] AP startup trampoline

## Per CPU

- [ ] GDT
- [ ] TSS
- [ ] IST stacks
- [ ] current task pointer
- [ ] runqueue
- [ ] scheduler state

Suggested:

- [ ] `*PopCPU`

## Synchronization

- [ ] atomic primitives
- [ ] spinlock
- [ ] IRQ-safe spinlock
- [ ] memory barriers
- [ ] per-CPU data

## Scheduler

- [ ] CPU-local queues
- [ ] task migration
- [ ] load balancing
- [ ] affinity capabilities eventually

---

# PHASE 31 — FPU/SIMD State

- [x] CPUID capabilities (`cpuid_get_features`, cpuinfo drive)
- [x] enable SSE (GRUB path sets CR0/CR4; UEFI inherits from firmware; IRQ/syscall stubs `fxsave64`/`fxrstor64`)
- [ ] enable XSAVE if available
- [ ] allocate per-task extended-state area
- [ ] save on switch
- [ ] restore on switch
- [ ] test XMM preservation
- [ ] test AVX preservation if supported

---

# PHASE 32 — Security Hardening

## Memory

- [ ] NX kernel data
- [ ] RO kernel text
- [ ] RO user text
- [ ] guard kernel stacks
- [ ] guard user stacks
- [ ] null page unmapped
- [ ] user cannot map kernel physical pages

## CPU

- [ ] SMEP
- [ ] SMAP
- [ ] UMIP if useful
- [ ] CR0.WP
- [x] NXE

## Syscalls

- [ ] validate all sizes
- [ ] validate arithmetic overflow
- [ ] validate every user pointer
- [ ] enforce capability rights

## Objects

- [ ] generation-count capability handles
- [ ] no stale handles
- [ ] cleanup on realm death
- [ ] revocation tests

---

# PHASE 33 — \*PopManifest Application Model

Don't make “ELF file = complete application identity.”

Create a manifest.

Suggested:

- [ ] `*PopManifest`

Fields might include:

- [ ] app ID
- [ ] version
- [ ] entry executable
- [ ] requested memory
- [ ] requested device classes
- [ ] requested network access
- [ ] requested persistent storage
- [ ] requested capabilities
- [ ] UI permissions
- [ ] dependency list

Kernel or launcher decides which resources to grant.

---

# PHASE 34 — \*PopPkg Package Format

Possible package:

```text
manifest
executables
libraries
resources
signature
metadata
```

Tasks:

- [ ] package parser
- [ ] package versioning
- [ ] integrity hash
- [ ] optional signatures
- [ ] application resources
- [ ] install location
- [ ] uninstall
- [ ] dependency metadata

Do not blindly clone `.deb`, RPM, Flatpak, or AppImage.

Learn from them, then make a Popcorn-native model.

---

# PHASE 35 — Application Launcher

Suggested:

- [ ] `*PopLaunch`

Responsibilities:

- [ ] load package manifest
- [ ] create realm
- [ ] allocate address space
- [ ] load ELF
- [ ] create initial capability table
- [ ] grant permitted devices
- [ ] construct startup context
- [ ] launch task
- [ ] observe process exit

---

# PHASE 36 — Namespaces Without Unix Assumptions

Decide whether Popcorn needs a universal path tree.

Possible Popcorn resource naming:

```text
dev:kbd0
dev:display0
fs:system
fs:user
net:default
service:graph
service:audio
```

Tasks:

- [ ] design `*PopName`
- [ ] lookup resource
- [ ] bind name to capability
- [ ] user-local namespaces
- [ ] application-local namespace
- [ ] compatibility `/dev` implemented by LibPop if desired

---

# PHASE 37 — Service Discovery

Suggested:

- [ ] `*PopHub`

Provide:

- [ ] register service
- [ ] unregister service
- [ ] find service
- [ ] receive service capability
- [ ] restrict service visibility
- [ ] detect dead service

Possible services:

```text
graph
audio
net
filesystem
clipboard
package
logging
```

---

# PHASE 38 — Logging and Diagnostics

Suggested:

- [ ] `*PopLog`

## Kernel

- [ ] structured kernel log
- [ ] severity
- [ ] subsystem
- [ ] timestamp
- [ ] CPU ID
- [ ] realm ID
- [ ] task ID

## Userspace

- [ ] logging service
- [ ] app logging
- [ ] crash reports
- [ ] fault reason
- [ ] capability violation reports

---

# PHASE 39 — Debugger Support

Suggested:

- [ ] `*PopDbg`

Implement:

- [ ] enumerate realms
- [ ] enumerate tasks
- [ ] inspect registers
- [ ] inspect mappings
- [ ] inspect capabilities
- [ ] suspend task
- [ ] resume task
- [ ] single-step later
- [ ] breakpoints later

---

# PHASE 40 — Boot Evolution

Keep current boot paths working.

- [x] GRUB path stays bootable
- [x] native UEFI path stays bootable
- [ ] kernel receives normalized boot-info structure
- [ ] firmware-specific logic ends before core initialization
- [x] boot path selects framebuffer
- [x] boot path supplies memory map
- [ ] boot path supplies ACPI root
- [ ] boot path supplies init package/device

Suggested handoff:

- [ ] `*PopBootInfo`

---

# PHASE 41 — Root/System Realm

Instead of assuming PID 1 semantics, define your own bootstrap realm.

Suggested:

- [ ] `*RootRealm`

It initially receives powerful capabilities:

- [ ] package/storage
- [ ] device manager
- [ ] display
- [ ] networking
- [ ] child realm creation
- [ ] resource delegation

Kernel remains more primitive.

---

# PHASE 42 — Resource Manager Policy

Exokernels still need resource arbitration.

But policy can live outside the kernel.

Suggested:

- [ ] `*PopBroker`

Possible responsibilities:

- [ ] decide memory quotas
- [ ] allocate CPU shares
- [ ] choose who gets devices
- [ ] reclaim resources
- [ ] respond to resource pressure

Kernel merely enforces decisions.

---

# PHASE 43 — CPU Scheduling as Exo Mechanism

Eventually split scheduler mechanism from policy.

Kernel provides:

- [ ] runnable task primitives
- [ ] CPU time accounting
- [ ] timer preemption
- [ ] safe task switch

Userspace can influence:

- [ ] priority hints
- [ ] deadline requests
- [ ] CPU affinity
- [ ] scheduling groups

Suggested API:

- [ ] `*PopSched`

Do not expose unsafe arbitrary CPU control.

---

# PHASE 44 — Persistent Storage Model

Rather than making the kernel understand every file:

- [ ] kernel owns raw device protection
- [ ] `*PopBlock` owns ranges
- [ ] filesystem receives ranges
- [ ] filesystem determines allocation policy
- [ ] application gets file/object capabilities

Potential future abstraction:

- [ ] `*PopObject`

Could represent durable named objects without forcing Unix inode semantics.

---

# PHASE 45 — Compatibility Personality

Only now decide how much familiar Unix-like behavior you actually want.

Suggested:

- [ ] `*PopCompat`

Possible compatibility features:

- [ ] argc/argv
- [ ] stdin/stdout/stderr
- [ ] fd numbers
- [ ] paths
- [ ] open/read/write
- [ ] directories
- [ ] pipes
- [ ] POSIX-ish errno
- [ ] sockets wrapper

This is a **personality**, not the operating system's fundamental architecture.

---

# PHASE 46 — Native Popcorn Application API

Build something that actually differentiates Popcorn.

Potential native primitives:

- [ ] capabilities instead of global handles
- [ ] explicit shared memory
- [ ] event-driven IPC
- [ ] direct resource negotiation
- [ ] declarative manifests
- [ ] per-app resource domains
- [ ] direct device capabilities for trusted apps
- [ ] capability-safe service discovery

Suggested user-facing API:

```c
PopCap display = pop_service("graph");
PopCap surface = pop_call(display, CREATE_SURFACE, ...);
PopCap timer   = pop_event_timer(...);
```

instead of forcing every program through Unix-style file semantics.

---

# PHASE 47 — Native SDK

Suggested:

- [ ] `*PopSDK`

Include:

- [ ] C headers
- [ ] Rust crate
- [ ] linker script
- [ ] startup runtime
- [ ] package builder
- [ ] manifest builder
- [ ] emulator runner
- [ ] debugger support

Command idea:

```text
pop build
pop run
pop package
pop debug
```

Names can change.

---

# PHASE 48 — Rust Native API

Create safe wrappers around raw capabilities.

Possible crate:

- [ ] `*popcorn`
- [ ] `*popcorn::mem`
- [ ] `*popcorn::task`
- [ ] `*popcorn::ipc`
- [ ] `*popcorn::event`
- [ ] `*popcorn::device`
- [ ] `*popcorn::graph`
- [ ] `*popcorn::net`

Ensure unsafe resource operations remain explicitly unsafe.

---

# PHASE 49 — C Native API

Provide a straightforward alternative:

```c
pop_cap_t
pop_event_t
pop_port_t
pop_result_t
```

- [ ] headers
- [ ] startup library
- [ ] syscall wrappers
- [ ] capability helpers
- [ ] IPC helpers
- [ ] event helpers

---

# PHASE 50 — Self-hosted Shell

Eventually move your shell out of Ring 0.

Suggested:

- [ ] `*PopShell`

Steps:

- [ ] keep existing kernel shell for debugging
- [ ] create user-mode shell
- [ ] launch ELF programs
- [ ] list services
- [ ] inspect resources
- [ ] filesystem operations
- [ ] task/realm monitoring
- [ ] package operations

Eventually:

- [ ] kernel shell becomes emergency/debug-only

---

# PHASE 51 — User-Space System Monitor

Suggested:

- [ ] `*PopScope`

Show:

- [ ] CPU usage
- [ ] memory ownership
- [ ] realms
- [ ] tasks
- [ ] capabilities
- [ ] events
- [ ] devices
- [ ] filesystem
- [ ] network
- [ ] logs

This will make debugging the ExoCore far easier.

---

# PHASE 52 — Audio

Eventually:

- [ ] discover audio hardware
- [ ] basic PCM output
- [ ] audio service
- [ ] shared buffers
- [ ] app streams
- [ ] mixer

Suggested:

- [ ] `*PopAudio`

Kernel should not need to understand “application volume.”

---

# PHASE 53 — Networking Service Ecosystem

Once `*PopNet` exists:

- [ ] DHCP client
- [ ] DNS resolver
- [ ] TCP service
- [ ] HTTPS/TLS library
- [ ] network permissions in manifests

Potential app model:

```text
App
 ↓
network capability
 ↓
*PopNet
```

Apps without network capability simply cannot access it.

---

# PHASE 54 — GUI Application Model

`*PopGraph` capabilities could grant:

- [ ] create surface
- [ ] resize surface
- [ ] submit framebuffer
- [ ] receive keyboard events
- [ ] receive pointer events

App does not receive raw framebuffer unless explicitly granted.

---

# PHASE 55 — Sandboxing

Capabilities make this much cleaner than Unix UID assumptions.

Profiles:

- [ ] no filesystem
- [ ] user files only
- [ ] no network
- [ ] one network endpoint
- [ ] display only
- [ ] no raw device access
- [ ] restricted CPU/memory budget

Suggested:

- [ ] `*PopProfile`

---

# PHASE 56 — Crash Isolation

- [ ] detect user fault
- [ ] terminate faulty task
- [ ] revoke capabilities
- [ ] destroy mappings
- [ ] notify parent/service
- [ ] restart service if policy says so
- [ ] preserve crash record

Kernel stays alive.

---

# PHASE 57 — Driver Crash Recovery

For userspace drivers:

- [ ] detect driver death
- [ ] mask IRQ
- [ ] revoke MMIO mapping
- [ ] revoke DMA mappings
- [ ] reset device if possible
- [ ] restart driver realm
- [ ] reconnect clients

This is a major advantage over a purely monolithic architecture.

---

# PHASE 58 — Updates / System Images

Design deliberately.

Possible model:

- [ ] read-only core system image
- [ ] writable user data
- [ ] atomic update slot
- [ ] rollback

Suggested:

- [ ] `*PopImage`

Avoid reinventing a mutable `/usr` unless you actually want it.

---

# PHASE 59 — Filesystem Strategy Decision

Choose your own model.

Potential long-term options:

- [ ] FAT32 boot/interchange only
- [ ] ext2 compatibility
- [ ] custom native filesystem
- [ ] object store
- [ ] log-structured filesystem
- [ ] userspace filesystem servers

If creating native FS:

Suggested:

- [ ] `*PopFS2`

But do this much later than Ring 3/ExoCore.

---

# PHASE 60 — Custom Native Filesystem, OPTIONAL

Only if it becomes a meaningful goal.

Features:

- [ ] 64-bit addressing
- [ ] journaling or copy-on-write
- [ ] checksums
- [ ] allocation bitmap/tree
- [ ] directories
- [ ] timestamps
- [ ] metadata
- [ ] sparse files
- [ ] atomic rename
- [ ] crash recovery

Do not build this merely because “OSes need filesystems.”

---

# PHASE 61 — Virtual Memory Advanced Features

After basic userspace:

- [ ] demand-zero
- [ ] guard regions
- [ ] copy-on-write
- [ ] lazy ELF loading
- [ ] shared memory
- [ ] memory-mapped files
- [ ] page aging
- [ ] swapping only if actually desired

Suggested resource:

- [ ] `*PopVMObject`

---

# PHASE 62 — Memory Pressure

Kernel mechanism:

- [ ] track physical pressure
- [ ] notify interested resource manager

Userspace policy:

- [ ] choose caches to evict
- [ ] choose realms to pressure
- [ ] choose pages to discard

Suggested event:

- [ ] `*POP_EVENT_MEMORY_PRESSURE`

---

# PHASE 63 — CPU Resource Contracts

Possible Popcorn-native design:

```text
Realm requests:
10% CPU minimum
50 ms deadline
2 cores max
```

Kernel validates/enforces mechanism.

Suggested:

- [ ] `*PopCPUGrant`

Later, not required for first ExoCore.

---

# PHASE 64 — Capability-Based Device Assignment

Example:

```text
*RootRealm owns NVMe controller
       ↓
grants block-range capability
       ↓
*PopFS
       ↓
grants file/object capability
       ↓
Application
```

Checklist:

- [ ] device ownership
- [ ] resource slicing
- [ ] derived capabilities
- [ ] revocation
- [ ] crash cleanup

---

# PHASE 65 — System Services

Potential native services:

- [ ] `*PopHub` — service discovery
- [ ] `*PopGraph` — graphics
- [ ] `*PopInput` — input routing
- [ ] `*PopAudio` — sound
- [ ] `*PopNet` — network
- [ ] `*PopFS` — filesystem
- [ ] `*PopPkg` — package manager
- [ ] `*PopLog` — logging
- [ ] `*PopBroker` — resource policy
- [ ] `*PopLaunch` — application launcher

None need to be kernel subsystems.

---

# PHASE 66 — Kernel Reduction Pass

Once userspace replacements work:

Review every subsystem still in Ring 0.

For each ask:

> Does this require privilege, or did it just start life in the kernel?

Candidates to remove/move:

- [ ] shell
- [ ] filesystem naming
- [ ] file descriptors
- [ ] heap policy
- [ ] block cache
- [ ] networking
- [ ] framebuffer policy
- [ ] package logic
- [ ] application launcher
- [ ] driver management

Keep only mechanisms where privilege/isolation matters.

---

# PHASE 67 — \*PopCore Final Responsibilities

Long-term Ring 0 should primarily own:

- [ ] CPU privilege setup
- [ ] interrupt entry
- [ ] physical memory protection
- [ ] page-table enforcement
- [ ] context switching
- [ ] scheduling mechanism
- [ ] capability enforcement
- [ ] resource accounting
- [ ] interrupt routing
- [ ] MMIO/I/O protection
- [ ] DMA/IOMMU protection
- [ ] IPC mechanism
- [ ] timers/events mechanism
- [ ] boot-critical fallback infrastructure

Everything else becomes negotiable.

---

# PHASE 68 — Compatibility Layer

Only after native design is clear.

Possible compatibility personalities:

- [ ] `*PopCompat`
- [ ] POSIX-ish libc compatibility
- [ ] simple Unix path semantics
- [ ] eventually Linux source compatibility for selected software

Avoid binary Linux compatibility until much later unless it becomes specifically useful.

Popcorn's native API should remain first-class.

---

# PHASE 69 — Application Ecosystem

Build several native programs specifically to prove that Popcorn has value independent of Linux.

Examples:

- [ ] terminal
- [ ] text editor
- [ ] system monitor
- [ ] file browser
- [ ] graphical demo
- [ ] image viewer
- [ ] network client
- [ ] development utility
- [ ] package browser

At least some should use native capability APIs rather than compatibility wrappers.

---

# PHASE 70 — Development Environment

Long-term:

- [ ] compile programs on Linux/macOS
- [ ] `pop run` in QEMU
- [ ] deploy to USB
- [ ] serial debugger
- [ ] userspace crash traces
- [ ] symbol files
- [ ] package builder
- [ ] emulator integration

Eventually:

- [ ] compile Popcorn software from Popcorn itself

---

# PHASE 71 — Self Hosting

Far-future milestone:

- [ ] filesystem can host source tree
- [ ] compiler/runtime available
- [ ] linker available
- [ ] build system available
- [ ] build small Popcorn app on Popcorn
- [ ] build `*LibPop` on Popcorn
- [ ] build kernel components on Popcorn

This is not necessary for a credible OS, but it is an excellent maturity milestone.

---

# PHASE 72 — v1.0 Definition

Do not define v1.0 as “has as many features as Linux.”

Define v1.0 around **your architecture**.

Suggested v1.0 requirements:

- [x] GRUB boot
- [x] native UEFI boot
- [ ] stable physical memory manager
- [ ] stable VMM
- [ ] stable preemptive scheduling
- [ ] Ring-3 isolation
- [ ] ELF applications
- [ ] user application runtime
- [ ] capability system
- [ ] capability delegation/revocation
- [ ] shared-memory IPC
- [ ] event mechanism
- [ ] ExoCore memory interface
- [ ] ExoCore resource interface
- [ ] `*LibPop`
- [ ] userspace shell
- [ ] userspace filesystem personality
- [ ] persistent storage
- [ ] network stack
- [ ] native application API
- [ ] package/application format
- [ ] real-hardware support on at least one target system
- [ ] no routine kernel crashes
- [ ] driver/application crash isolation
- [ ] documented `*PopABI`
- [ ] documented `*ExoABI`
- [ ] SDK capable of building third-party native programs

---

# YOUR CURRENT CRITICAL PATH

Do not look at all 72 phases every day.

For **right now**, your queue is:

- [ ] **1. Fix real-hardware storage writes**
- [x] **2. Fix PIC cascade configuration**
- [x] **3. Fix PIT polling time accounting** (code fixed: poll mode emits one tick per PIT period like IRQ0; runtime measurements in Phase 2.2 still open)
- [x] **4. Verify `-mno-red-zone`**
- [x] **5. Build one normalized `*PopFrame` interrupt frame**
- [x] **6. Normalize ASM interrupt entry** (one `isr_common` for all 256 vectors)
- [x] **7. Fix stack alignment before ASM → C calls**
- [x] **8. Move timer preemption to interrupt-frame context switching** (also real IRQ0 on UEFI via the LAPIC virtual wire)
- [x] **9. Add `*PopTaskEntry` trampoline** (`task_entry_trampoline` → `scheduler_task_entry`, data passed in RDI)
- [x] **10. Stress-test kernel multitasking** (`mon -stress` + 60 s soak in `test-shell`; 10-minute / 1-hour runs still open)
- [ ] **11. Clean GDT selector definitions**
- [ ] **12. Add user code/data descriptors**
- [ ] **13. Create separate per-task kernel stacks**
- [ ] **14. Update `TSS.RSP0` during switches**
- [ ] **15. Implement user-page mapping**
- [ ] **16. Hard-code one Ring-3 test program**
- [ ] **17. `iretq` into CPL3**
- [ ] **18. Make a user protection fault kill only the user task**
- [ ] **19. Repair `int 0x80`**
- [ ] **20. Round-trip Ring 3 → Ring 0 → Ring 3**
- [ ] **21. Implement `*PopCopyIn` / `*PopCopyOut`**
- [ ] **22. Build `*PopRealm`**
- [ ] **23. Load ELF64 from FAT32**
- [ ] **24. Run two isolated ELF programs**
- [ ] **25. Build minimal `*LibPop`**
- [ ] **26. Introduce `*PopCap`**
- [ ] **27. Put memory behind capabilities**
- [ ] **28. Implement `*PopEvent`**
- [ ] **29. Implement `*PopPort` IPC**
- [ ] **30. Implement capability delegation**
- [ ] **31. Implement capability revocation**
- [ ] **32. Freeze `*ExoABI v0`**
- [ ] **33. Move malloc policy into `*LibPop`**
- [ ] **34. Move FD policy into `*LibPop`**
- [ ] **35. Give filesystem raw `*PopBlock` resources**
- [ ] **36. Start moving filesystem policy to Ring 3**
- [ ] **37. Introduce service discovery**
- [ ] **38. Move shell to Ring 3**
- [ ] **39. Add ACPI/APIC**
- [ ] **40. Expose safe IRQ/MMIO capabilities**
- [ ] **41. Move one noncritical driver into Ring 3**
- [ ] **42. Build `*PopNet`**
- [ ] **43. Build `*PopGraph`**
- [ ] **44. Build `*PopPkg` / `*PopManifest`**
- [ ] **45. Reduce Ring-0 code into final `*PopCore`**

---

# The milestones to remember

When overwhelmed, ignore the whole document and remember only this:

```text
CURRENT POPCORN
      │
      ▼
Hardware reliable
      │
      ▼
Interrupts correct
      │
      ▼
Context switching correct
      │
      ▼
Ring 3
      │
      ▼
Safe syscalls
      │
      ▼
ELF applications
      │
      ▼
Process/realm isolation
      │
      ▼
Capabilities
      │
      ▼
Memory as a resource
      │
      ▼
Events + IPC
      │
      ▼
*ExoABI
      │
      ▼
*LibPop
      │
      ▼
Policy moves out of kernel
      │
      ▼
Userspace services/drivers
      │
      ▼
*PopCore
```

The point where Popcorn becomes meaningfully different from a traditional monolithic hobby OS is not merely Ring 3.

It is this transition:

```text
Traditional kernel:

"Tell me what operation you want and
I will perform the OS policy for you."

             ↓

Popcorn ExoCore:

"Here are the protected resources you own.
Build the abstraction you want."
```

That should remain the architectural test for every major feature you add.
