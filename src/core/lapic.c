/* src/core/lapic.c - LAPIC virtual-wire mode + IOAPIC masking (see lapic.h). */
#include "../includes/lapic.h"
#include "../includes/boot_fb.h"
#include <stdint.h>

#define IA32_APIC_BASE_MSR 0x1Bu
#define APIC_BASE_ENABLE   (1u << 11)

#define LAPIC_ID       0x020u
#define LAPIC_TPR      0x080u
#define LAPIC_EOI      0x0B0u
#define LAPIC_SVR      0x0F0u
#define LAPIC_LVT_TIMER 0x320u
#define LAPIC_LVT_LINT0 0x350u
#define LAPIC_LVT_LINT1 0x360u
#define LAPIC_LVT_ERROR 0x370u
#define LAPIC_TIMER_INIT 0x380u

#define LVT_MASKED     (1u << 16)
#define LVT_DM_NMI     (4u << 8)
#define LVT_DM_EXTINT  (7u << 8)
#define SVR_ENABLE     (1u << 8)
#define SPURIOUS_VECTOR 0xFFu

/* Default IOAPIC address (ACPI MADT would be authoritative; every PC chipset and
 * QEMU q35 put the first IOAPIC here). */
#define IOAPIC_DEFAULT_PHYS 0xFEC00000ULL
#define IOAPIC_REG_VER 0x01u
#define IOAPIC_REDTBL(n) (0x10u + 2u * (n))

static volatile uint32_t* g_lapic;  /* identity-mapped, uncached */
static uint64_t g_lapic_phys;
static uint32_t g_ioapic_masked;

static uint64_t rdmsr(uint32_t msr) {
    uint32_t lo, hi;
    __asm__ volatile("rdmsr" : "=a"(lo), "=d"(hi) : "c"(msr));
    return ((uint64_t)hi << 32) | lo;
}

static bool cpu_has_apic(void) {
    uint32_t a = 1, b, c = 0, d;
    __asm__ volatile("cpuid" : "+a"(a), "=b"(b), "+c"(c), "=d"(d));
    (void)b;
    return (d & (1u << 9)) != 0;
}

static uint32_t lapic_read(uint32_t reg) {
    return g_lapic[reg / 4u];
}

static void lapic_write(uint32_t reg, uint32_t val) {
    g_lapic[reg / 4u] = val;
    (void)g_lapic[LAPIC_ID / 4u]; /* posted write: read back to flush */
}

static void ioapic_mask_all(void) {
    volatile uint32_t* io = (volatile uint32_t*)(uintptr_t)IOAPIC_DEFAULT_PHYS;
    boot_identity_uncache_range(IOAPIC_DEFAULT_PHYS, 0x1000);
    io[0] = IOAPIC_REG_VER;
    uint32_t ver = io[4];
    if (ver == 0xFFFFFFFFu || ver == 0) {
        return; /* no IOAPIC decoded here */
    }
    uint32_t entries = ((ver >> 16) & 0xFFu) + 1u;
    if (entries > 240u) {
        return; /* implausible: not an IOAPIC */
    }
    for (uint32_t n = 0; n < entries; n++) {
        io[0] = IOAPIC_REDTBL(n);
        io[4] = io[4] | LVT_MASKED; /* bit 16 = mask in a redirection entry too */
    }
    g_ioapic_masked = entries;
}

void lapic_setup_virtual_wire(void) {
    ioapic_mask_all();

    if (!cpu_has_apic()) {
        return;
    }
    uint64_t base = rdmsr(IA32_APIC_BASE_MSR);
    if (!(base & APIC_BASE_ENABLE)) {
        /* Globally disabled LAPIC: LINT0/LINT1 act as plain INTR/NMI pins, so the PIC
         * already reaches the CPU directly. */
        return;
    }
    g_lapic_phys = base & 0x000FFFFFFFFFF000ULL;
    if (g_lapic_phys == 0 || g_lapic_phys >= (64ULL << 30)) {
        return; /* outside the identity map */
    }
    boot_identity_uncache_range(g_lapic_phys, 0x1000);
    g_lapic = (volatile uint32_t*)(uintptr_t)g_lapic_phys;

    /* Firmware's LAPIC timer (OVMF ticks on it) must not keep firing into our IDT. */
    lapic_write(LAPIC_LVT_TIMER, LVT_MASKED);
    lapic_write(LAPIC_TIMER_INIT, 0);
    lapic_write(LAPIC_LVT_ERROR, LVT_MASKED);
    lapic_write(LAPIC_LVT_LINT0, LVT_DM_EXTINT);       /* 8259 INTR, unmasked */
    lapic_write(LAPIC_LVT_LINT1, LVT_DM_NMI);
    lapic_write(LAPIC_TPR, 0);
    lapic_write(LAPIC_SVR, SVR_ENABLE | SPURIOUS_VECTOR);
    /* Retire anything firmware left in service (one EOI per in-service vector). */
    for (int i = 0; i < 16; i++) {
        lapic_write(LAPIC_EOI, 0);
    }
}

/* Fixed-delivery IPI to this CPU (ICR shorthand "self"). Returns -1 without a LAPIC. */
int lapic_self_ipi(uint8_t vector) {
    if (!g_lapic) {
        return -1;
    }
    for (int i = 0; i < 100000 && (lapic_read(0x300) & (1u << 12)); i++) {
        __asm__ volatile("pause");
    }
    lapic_write(0x310, 0);
    lapic_write(0x300, (uint32_t)vector | (1u << 14) | (1u << 18)); /* assert, self */
    return 0;
}

bool lapic_active(void) {
    return g_lapic != 0;
}

void lapic_eoi(void) {
    if (g_lapic) {
        lapic_write(LAPIC_EOI, 0);
    }
}

uint32_t lapic_id(void) {
    return g_lapic ? (lapic_read(LAPIC_ID) >> 24) : 0;
}

uint64_t lapic_base_phys(void) {
    return g_lapic_phys;
}

uint32_t ioapic_masked_entries(void) {
    return g_ioapic_masked;
}
