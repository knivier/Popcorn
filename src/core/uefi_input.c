#include "../includes/uefi_input.h"
#include "../includes/uefi_boot.h"
#include "../includes/multiboot2.h"
#include <stddef.h>
#include <stdint.h>

#define EFIAPI __attribute__((ms_abi))
#define EFI_SUCCESS          0ULL
#define EFI_NOT_READY        0x8000000000000007ULL

typedef struct {
    uint16_t ScanCode;
    uint16_t UnicodeChar;
} EFI_INPUT_KEY;

typedef uint64_t (EFIAPI *EFI_INPUT_READ_KEY)(void* This, EFI_INPUT_KEY* Key);
typedef uint64_t (EFIAPI *EFI_INPUT_READ_KEY_EX)(void* This, EFI_INPUT_KEY* Key, uint64_t* KeyState);
typedef struct {
    void* Reset;
    EFI_INPUT_READ_KEY ReadKeyStroke;
    void* WaitForKey;
} EFI_SIMPLE_TEXT_INPUT_PROTOCOL;

typedef struct {
    void* Reset;
    EFI_INPUT_READ_KEY_EX ReadKeyStrokeEx;
    void* WaitForKeyEx;
    void* SetState;
    void* RegisterKeyNotify;
    void* UnregisterKeyNotify;
} EFI_SIMPLE_TEXT_INPUT_EX_PROTOCOL;

static volatile PopcornUefiBootInfo* handoff(void) {
    return (volatile PopcornUefiBootInfo*)(uintptr_t)POPCORN_UEFI_HANDOFF_PHYS;
}

static uint64_t uefi_poll_attempts;

uint64_t uefi_input_poll_attempts(void) {
    return uefi_poll_attempts;
}

static uint64_t read_cr3(void) {
    uint64_t v;
    __asm__ volatile("mov %%cr3, %0" : "=r"(v) : : "memory");
    return v;
}

static void write_cr3(uint64_t v) {
    __asm__ volatile("mov %0, %%cr3" : : "r"(v) : "memory");
}

static uint64_t irq_save_cli(void) {
    uint64_t flags;
    __asm__ volatile("pushfq; pop %0; cli" : "=r"(flags) : : "memory");
    return flags;
}

static void irq_restore(uint64_t flags) {
    __asm__ volatile("push %0; popfq" : : "r"(flags) : "memory", "cc");
}

bool uefi_input_available(void) {
    if (!multiboot2_is_uefi_boot()) {
        return false;
    }
    volatile PopcornUefiBootInfo* h = handoff();
    return h && (h->flags & POPCORN_UEFI_FLAG_FIRMWARE_KBD) && h->conin != 0 &&
           h->firmware_cr3 != 0;
}

static uint64_t firmware_call_read_key(volatile PopcornUefiBootInfo* h, EFI_INPUT_KEY* key) {
    uint64_t fw_cr3 = h->firmware_cr3;
    uint64_t cur = read_cr3();
    uint64_t status = EFI_NOT_READY;

    uefi_poll_attempts++;

    if (!key || fw_cr3 == 0) {
        return EFI_NOT_READY;
    }

    /* Block IRQ delivery while CR3 points at firmware page tables (timer IRQ + fw CR3 hangs). */
    uint64_t flags = irq_save_cli();
    write_cr3(fw_cr3);

    if (h->flags & POPCORN_UEFI_FLAG_INPUT_EX) {
        EFI_SIMPLE_TEXT_INPUT_EX_PROTOCOL* ex =
            (EFI_SIMPLE_TEXT_INPUT_EX_PROTOCOL*)(uintptr_t)h->conin;
        if (ex && ex->ReadKeyStrokeEx) {
            status = ex->ReadKeyStrokeEx(ex, key, NULL);
        }
    } else {
        EFI_SIMPLE_TEXT_INPUT_PROTOCOL* con =
            (EFI_SIMPLE_TEXT_INPUT_PROTOCOL*)(uintptr_t)h->conin;
        if (con && con->ReadKeyStroke) {
            status = con->ReadKeyStroke(con, key);
        }
    }

    write_cr3(cur);
    irq_restore(flags);
    return status;
}

static uint8_t scan_to_ps2(uint16_t scan) {
    switch (scan) {
        case 0x0001: return 0x48;
        case 0x0002: return 0x50;
        case 0x0003: return 0x4D;
        case 0x0004: return 0x4B;
        case 0x0009: return 0x49;
        case 0x000A: return 0x51;
        case 0x0008: return 0x53;
        case 0x0017: return 0x1C;
        default: return 0;
    }
}

bool uefi_input_poll(uint8_t* key_out, bool* is_scancode) {
    EFI_INPUT_KEY key = {0, 0};
    uint64_t status;

    if (!key_out || !is_scancode || !uefi_input_available()) {
        return false;
    }

    status = firmware_call_read_key(handoff(), &key);
    if (status == EFI_NOT_READY) {
        return false;
    }
    if (status != EFI_SUCCESS) {
        return false;
    }

    if (key.ScanCode != 0) {
        uint8_t sc = scan_to_ps2(key.ScanCode);
        if (sc == 0) {
            return false;
        }
        *key_out = sc;
        *is_scancode = true;
        return true;
    }

    if (key.UnicodeChar != 0) {
        uint16_t ch = key.UnicodeChar;
        if (ch == 0x000Du || ch == 0x000Au) {
            *key_out = 0x1C;
            *is_scancode = true;
            return true;
        }
        if (ch == 0x0008u || ch == 0x007Fu) {
            *key_out = 0x0E;
            *is_scancode = true;
            return true;
        }
        if (ch == 0x0009u) {
            *key_out = 0x0F;
            *is_scancode = true;
            return true;
        }
        if (ch >= 32u && ch < 127u) {
            *key_out = (uint8_t)ch;
            *is_scancode = false;
            return true;
        }
        return false;
    }

    return false;
}
