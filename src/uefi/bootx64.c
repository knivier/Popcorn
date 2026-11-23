/*
 * Popcorn native UEFI loader (BOOTX64.EFI).
 * Loads /boot/kernel ELF segments and jumps to the kernel symbol uefi_start
 * (looked up in the ELF symbol table; not e_entry, which is the GRUB `start`).
 */
#include "../includes/uefi_boot.h"
#include <stdint.h>

typedef uint8_t  BOOLEAN;
typedef uint8_t  UINT8;
typedef int8_t   INT8;
typedef int64_t  INT64;
typedef uint16_t UINT16;
typedef uint32_t UINT32;
typedef uint64_t UINT64;
typedef uint64_t UINTN;
typedef UINT64   EFI_STATUS;
typedef void*    EFI_HANDLE;
typedef void*    EFI_EVENT;
typedef uint64_t EFI_PHYSICAL_ADDRESS;
typedef uint16_t CHAR16;
typedef CHAR16*  CHAR16_P;

#define EFIAPI __attribute__((ms_abi))
#ifndef TRUE
#define TRUE  1
#define FALSE 0
#endif
#ifndef NULL
#define NULL ((void*)0)
#endif
#define EFI_SUCCESS               0
#define EFI_LOAD_ERROR            0x8000000000000001ULL
#define EFI_INVALID_PARAMETER     0x8000000000000002ULL
#define EFI_NOT_FOUND             0x800000000000000eULL
#define EFI_BUFFER_TOO_SMALL      0x8000000000000005ULL
#define EFI_ERROR(x)              ((INT64)(x) < 0)

#define EFI_OPEN_PROTOCOL_BY_HANDLE_PROTOCOL 0x00000001
#define EFI_LOADER_DATA                      0x00000002
#define EFI_ALLOCATE_ANY_PAGES               0x00000000
#define EFI_ALLOCATE_MAX_ADDRESS             0x00000001
#define EFI_ALLOCATE_ADDRESS                 0x00000002
#define EFI_MEMORY_WB                        0x00000006
#define EFI_MMAP_PAGES                       8
#define PT_LOAD                              1
/* EFI_GRAPHICS_PIXEL_FORMAT (UEFI spec 12.9). */
#define GOP_PIXEL_RGB_8BIT_PER_CHANNEL       0 /* PixelRedGreenBlueReserved8BitPerColor */
#define GOP_PIXEL_BGR_8BIT_PER_CHANNEL       1 /* PixelBlueGreenRedReserved8BitPerColor */
#define GOP_PIXEL_BIT_MASK                   2 /* PixelBitMask: layout in PixelInformation */
#define GOP_PIXEL_BLT_ONLY                   3 /* PixelBltOnly: no linear framebuffer */
#define MAX_KERNEL_FILE_BYTES                (16U * 1024U * 1024U)
/* Kernel physical span sanity limits (link.ld loads at 1 MiB). */
#define KERNEL_SPAN_MIN_PHYS                 0x100000ULL
#define KERNEL_SPAN_MAX_BYTES                (256ULL * 1024ULL * 1024ULL)
/* The kernel reads the MBI through its low identity map; keep it below 4 GiB. */
#define MBI_MAX_PHYS                         0xFFFFFFFFULL
#define MBI_MIN_PAGES                        16u /* 64 KiB */

typedef struct {
    UINT32 Data1;
    UINT16 Data2;
    UINT16 Data3;
    UINT8  Data4[8];
} EFI_GUID;

static const EFI_GUID gEfiLoadedImageGuid =
    {0x5B1B31A1, 0x9562, 0x11d2, {0x8E, 0x3F, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B}};
static const EFI_GUID gEfiBlockIoProtocolGuid =
    {0x964E5B21, 0x6459, 0x11d2, {0x8E, 0x39, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B}};
static const EFI_GUID gEfiSimpleFileSystemGuid =
    {0x964E5B22, 0x6459, 0x11d2, {0x8E, 0x39, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B}};
static const EFI_GUID gEfiGraphicsOutputProtocolGuid =
    {0x9042a9de, 0x23dc, 0x4a38, {0x96, 0xFB, 0x7A, 0xDE, 0xD0, 0x80, 0x51, 0x6A}};
static const EFI_GUID gEfiSimpleTextInputExProtocolGuid =
    {0x347790cb, 0x2612, 0x4ac9, {0x98, 0x23, 0x48, 0x88, 0x57, 0x0b, 0x25, 0x2f}};

typedef struct {
    UINT64 Signature;
    UINT32 Revision;
    UINT32 HeaderSize;
    UINT32 CRC32;
    UINT32 Reserved;
} EFI_TABLE_HEADER;

typedef struct {
    UINT32 Type;
    UINT32 Pad;
    EFI_PHYSICAL_ADDRESS PhysicalStart;
    UINT64 VirtualStart;
    UINT64 NumberOfPages;
    UINT64 Attribute;
} EFI_MEMORY_DESCRIPTOR;

typedef EFI_STATUS (EFIAPI *EFI_LOCATE_PROTOCOL)(const EFI_GUID* Protocol, void* Registration, void** Interface);
typedef EFI_STATUS (EFIAPI *EFI_HANDLE_PROTOCOL)(EFI_HANDLE Handle, const EFI_GUID* Protocol, void** Interface);
typedef EFI_STATUS (EFIAPI *EFI_LOCATE_HANDLE)(UINT32 SearchType, EFI_GUID* Protocol, void* SearchKey,
    UINTN* BufferSize, EFI_HANDLE* Buffer);
#define EFI_LOCATE_BY_PROTOCOL 2
typedef EFI_STATUS (EFIAPI *EFI_EXIT_BOOT_SERVICES)(EFI_HANDLE ImageHandle, UINTN MapKey);
typedef EFI_STATUS (EFIAPI *EFI_STALL)(UINTN Microseconds);
typedef EFI_STATUS (EFIAPI *EFI_GET_MEMORY_MAP)(UINTN* MemoryMapSize, EFI_MEMORY_DESCRIPTOR* MemoryMap,
    UINTN* MapKey, UINTN* DescriptorSize, UINT32* DescriptorVersion);
typedef EFI_STATUS (EFIAPI *EFI_ALLOCATE_POOL)(UINTN PoolType, UINTN Size, void** Buffer);
typedef EFI_STATUS (EFIAPI *EFI_FREE_POOL)(void* Buffer);
typedef EFI_STATUS (EFIAPI *EFI_ALLOCATE_PAGES)(UINTN Type, UINTN MemoryType, UINTN Pages, EFI_PHYSICAL_ADDRESS* Memory);
typedef EFI_STATUS (EFIAPI *EFI_FREE_PAGES)(EFI_PHYSICAL_ADDRESS Memory, UINTN Pages);

typedef struct {
    EFI_TABLE_HEADER Hdr;
    void* RaiseTPL;
    void* RestoreTPL;
    EFI_ALLOCATE_PAGES AllocatePages;
    EFI_FREE_PAGES FreePages;
    EFI_GET_MEMORY_MAP GetMemoryMap;
    EFI_ALLOCATE_POOL AllocatePool;
    EFI_FREE_POOL FreePool;
    void* CreateEvent;
    void* SetTimer;
    void* WaitForEvent;
    void* SignalEvent;
    void* CloseEvent;
    void* CheckEvent;
    void* InstallProtocolInterface;
    void* ReinstallProtocolInterface;
    void* UninstallProtocolInterface;
    EFI_HANDLE_PROTOCOL HandleProtocol;
    void* Reserved;
    void* RegisterProtocolNotify;
    EFI_LOCATE_HANDLE LocateHandle;
    void* LocateDevicePath;
    void* InstallConfigurationTable;
    void* LoadImage;
    void* StartImage;
    void* Exit;
    void* UnloadImage;
    EFI_EXIT_BOOT_SERVICES ExitBootServices;
    void* GetNextMonotonicCount;
    EFI_STALL Stall;
    void* SetWatchdogTimer;
    void* ConnectController;
    void* DisconnectController;
    void* OpenProtocol;
    void* CloseProtocol;
    void* OpenProtocolInformation;
    void* ProtocolsPerHandle;
    void* LocateHandleBuffer;
    EFI_LOCATE_PROTOCOL LocateProtocol;
} EFI_BOOT_SERVICES_PARTIAL;

typedef struct {
    UINT32 Version;
    UINT32 HorizontalResolution;
    UINT32 VerticalResolution;
    UINT32 PixelFormat;
    UINT32 PixelInformation[4];
    UINT32 PixelsPerScanLine;
} EFI_GRAPHICS_OUTPUT_MODE_INFORMATION;

typedef struct {
    UINT32 MaxMode;
    UINT32 Mode;
    EFI_GRAPHICS_OUTPUT_MODE_INFORMATION* Info;
    UINTN SizeOfInfo;
    EFI_PHYSICAL_ADDRESS FrameBufferBase;
    UINTN FrameBufferSize;
} EFI_GRAPHICS_OUTPUT_PROTOCOL_MODE;

typedef EFI_STATUS (EFIAPI *EFI_GOP_QUERY_MODE)(void* This, UINT32 ModeNumber, UINTN* SizeOfInfo,
    EFI_GRAPHICS_OUTPUT_MODE_INFORMATION** Info);
typedef EFI_STATUS (EFIAPI *EFI_GOP_SET_MODE)(void* This, UINT32 ModeNumber);

typedef struct {
    void* QueryMode;
    void* SetMode;
    void* Blt;
    EFI_GRAPHICS_OUTPUT_PROTOCOL_MODE* Mode;
} EFI_GRAPHICS_OUTPUT_PROTOCOL;

typedef struct _EFI_FILE_PROTOCOL EFI_FILE_PROTOCOL;

typedef EFI_STATUS (EFIAPI *EFI_FILE_OPEN)(EFI_FILE_PROTOCOL* Self, EFI_FILE_PROTOCOL** NewHandle,
    CHAR16_P FileName, UINT64 OpenMode, UINT64 Attributes);
typedef EFI_STATUS (EFIAPI *EFI_FILE_READ)(EFI_FILE_PROTOCOL* Self, UINTN* BufferSize, void* Buffer);
typedef EFI_STATUS (EFIAPI *EFI_FILE_CLOSE)(EFI_FILE_PROTOCOL* Self);
typedef EFI_STATUS (EFIAPI *EFI_FILE_GET_INFO)(EFI_FILE_PROTOCOL* Self, void* Information,
    UINTN* BufferSize);

typedef struct _EFI_FILE_PROTOCOL {
    UINT64 Revision;
    EFI_FILE_OPEN Open;
    EFI_FILE_CLOSE Close;
    void* Delete;
    EFI_FILE_READ Read;
    void* Write;
    void* GetPosition;
    void* SetPosition;
    EFI_FILE_GET_INFO GetInfo;
    void* SetInfo;
    void* Flush;
} EFI_FILE_PROTOCOL;

typedef EFI_STATUS (EFIAPI *EFI_SIMPLE_FILE_SYSTEM_OPEN_VOLUME)(void* This, EFI_FILE_PROTOCOL** Root);

typedef struct {
    UINT64 Revision;
    EFI_SIMPLE_FILE_SYSTEM_OPEN_VOLUME OpenVolume;
} EFI_SIMPLE_FILE_SYSTEM_PROTOCOL;

typedef EFI_STATUS (EFIAPI *EFI_TEXT_RESET)(void* This, BOOLEAN ExtendedVerification);
typedef EFI_STATUS (EFIAPI *EFI_TEXT_STRING)(void* This, const CHAR16* String);
typedef EFI_STATUS (EFIAPI *EFI_TEXT_CLEAR_SCREEN)(void* This);

typedef struct {
    EFI_TEXT_RESET Reset;
    EFI_TEXT_STRING OutputString;
    void* TestString;
    void* QueryMode;
    void* SetMode;
    void* SetAttribute;
    EFI_TEXT_CLEAR_SCREEN ClearScreen;
    void* SetCursorPosition;
    void* EnableCursor;
    void* Mode;
} EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL;

typedef struct {
    EFI_TABLE_HEADER Hdr;
    CHAR16* FirmwareVendor;
    UINT32 FirmwareRevision;
    EFI_HANDLE ConsoleInHandle;
    void* ConIn;
    EFI_HANDLE ConsoleOutHandle;
    EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL* ConOut;
    EFI_HANDLE StandardErrorHandle;
    void* StdErr;
    void* RuntimeServices;
    EFI_BOOT_SERVICES_PARTIAL* BootServices;
    UINTN NumberOfTableEntries;
    void* ConfigurationTable;
} EFI_SYSTEM_TABLE;

typedef struct {
    UINT64 Revision;
    EFI_HANDLE ParentHandle;
    EFI_SYSTEM_TABLE* SystemTable;
    EFI_HANDLE DeviceHandle;
    void* FilePath;
    void* Reserved;
    UINT32 LoadOptionsSize;
    void* LoadOptions;
} EFI_LOADED_IMAGE_PROTOCOL;

typedef struct {
    UINT32 p_type;
    UINT32 p_flags;
    UINT64 p_offset;
    UINT64 p_vaddr;
    UINT64 p_paddr;
    UINT64 p_filesz;
    UINT64 p_memsz;
    UINT64 p_align;
} Elf64_Phdr;

typedef struct {
    UINT32 sh_name;
    UINT32 sh_type;
    UINT64 sh_flags;
    UINT64 sh_addr;
    UINT64 sh_offset;
    UINT64 sh_size;
    UINT32 sh_link;
    UINT32 sh_info;
    UINT64 sh_addralign;
    UINT64 sh_entsize;
} Elf64_Shdr;

typedef struct {
    UINT32 st_name;
    UINT8  st_info;
    UINT8  st_other;
    UINT16 st_shndx;
    UINT64 st_value;
    UINT64 st_size;
} Elf64_Sym;

typedef struct {
    UINT8  e_ident[16];
    UINT16 e_type;
    UINT16 e_machine;
    UINT32 e_version;
    UINT64 e_entry;
    UINT64 e_phoff;
    UINT64 e_shoff;
    UINT32 e_flags;
    UINT16 e_ehsize;
    UINT16 e_phentsize;
    UINT16 e_phnum;
    UINT16 e_shentsize;
    UINT16 e_shnum;
    UINT16 e_shstrndx;
} Elf64_Ehdr;

static void print(EFI_SYSTEM_TABLE* st, const CHAR16* s) {
    if (st && st->ConOut) {
        st->ConOut->OutputString(st->ConOut, s);
    }
}

static void print_err(EFI_SYSTEM_TABLE* st, const CHAR16* s) {
    print(st, L"\r\n[Popcorn UEFI] ");
    print(st, s);
    print(st, L"\r\n");
}

static void* memcpy_local(void* dst, const void* src, UINTN n) {
    UINT8* d = (UINT8*)dst;
    const UINT8* s = (const UINT8*)src;
    for (UINTN i = 0; i < n; i++) {
        d[i] = s[i];
    }
    return dst;
}

static void* memset_local(void* dst, INT8 v, UINTN n) {
    UINT8* d = (UINT8*)dst;
    for (UINTN i = 0; i < n; i++) {
        d[i] = (UINT8)v;
    }
    return dst;
}

typedef EFI_STATUS (EFIAPI *EFI_GOP_QUERY_MODE_FN)(void* This, UINT32 ModeNumber, UINTN* SizeOfInfo,
    EFI_GRAPHICS_OUTPUT_MODE_INFORMATION** Info);
typedef EFI_STATUS (EFIAPI *EFI_GOP_SET_MODE_FN)(void* This, UINT32 ModeNumber);

static BOOLEAN uefi_ptr_ok(UINTN p) {
    return p >= 0x1000ULL && p < 0xFFFFFFFFFFFFULL;
}

/* Prefer 1280×720 (compact VNC); else closest mode at/above 800×600. */
static void gop_prefer_720p(EFI_GRAPHICS_OUTPUT_PROTOCOL* gop) {
    EFI_GRAPHICS_OUTPUT_PROTOCOL_MODE* pm;
    EFI_GOP_QUERY_MODE_FN query;
    EFI_GOP_SET_MODE_FN set_mode;
    UINT32 best = 0xFFFFFFFFu;
    UINT32 best_score = 0xFFFFFFFFu;
    UINT32 i;

    if (!gop || !gop->Mode || !gop->QueryMode || !gop->SetMode) {
        return;
    }
    pm = gop->Mode;
    query = (EFI_GOP_QUERY_MODE_FN)gop->QueryMode;
    set_mode = (EFI_GOP_SET_MODE_FN)gop->SetMode;

    for (i = 0; i < pm->MaxMode; i++) {
        UINTN info_size = 0;
        EFI_GRAPHICS_OUTPUT_MODE_INFORMATION* mi = NULL;
        UINT32 score;
        UINT32 dx;
        UINT32 dy;
        if (EFI_ERROR(query(gop, i, &info_size, &mi)) || !mi) {
            continue;
        }
        /* Never switch to a mode without a usable linear framebuffer. */
        if (mi->PixelFormat > GOP_PIXEL_BIT_MASK) {
            continue;
        }
        if (mi->HorizontalResolution == 1280 && mi->VerticalResolution == 720) {
            best = i;
            best_score = 0;
            break;
        }
        if (mi->HorizontalResolution < 800 || mi->VerticalResolution < 600) {
            continue;
        }
        /* Prefer modes near 720p over jumping to a huge 1080p. */
        dx = (mi->HorizontalResolution > 1280)
                 ? (mi->HorizontalResolution - 1280)
                 : (1280 - mi->HorizontalResolution);
        dy = (mi->VerticalResolution > 720)
                 ? (mi->VerticalResolution - 720)
                 : (720 - mi->VerticalResolution);
        score = dx + dy;
        if (score < best_score) {
            best_score = score;
            best = i;
        }
    }
    if (best != 0xFFFFFFFFu && best != pm->Mode) {
        (void)set_mode(gop, best);
    }
}

/* Bit position of the lowest set bit of a GOP PixelBitMask channel mask. */
static UINT8 mask_shift(UINT32 m) {
    UINT8 s = 0;
    if (m == 0) {
        return 0;
    }
    while ((m & 1u) == 0) {
        m >>= 1;
        s++;
    }
    return s;
}

/* Number of set bits in a channel mask (channel width). */
static UINT8 mask_width(UINT32 m) {
    UINT8 n = 0;
    while (m) {
        n += (UINT8)(m & 1u);
        m >>= 1;
    }
    return n;
}

static BOOLEAN fill_boot_info_from_gop(EFI_SYSTEM_TABLE* st, PopcornUefiBootInfo* info) {
    EFI_BOOT_SERVICES_PARTIAL* bs = st->BootServices;
    EFI_GRAPHICS_OUTPUT_PROTOCOL* gop = NULL;
    EFI_STATUS status = bs->LocateProtocol(&gEfiGraphicsOutputProtocolGuid, NULL, (void**)&gop);
    if (EFI_ERROR(status) || !gop || !uefi_ptr_ok((UINTN)gop)) {
        return FALSE;
    }

    gop_prefer_720p(gop);

    EFI_GRAPHICS_OUTPUT_PROTOCOL_MODE* pm = gop->Mode;
    if (!pm || !uefi_ptr_ok((UINTN)pm)) {
        return FALSE;
    }

    EFI_GRAPHICS_OUTPUT_MODE_INFORMATION* mi = NULL;
    if (uefi_ptr_ok((UINTN)pm->Info)) {
        mi = pm->Info;
    } else if (gop->QueryMode) {
        EFI_GOP_QUERY_MODE_FN query = (EFI_GOP_QUERY_MODE_FN)gop->QueryMode;
        UINTN info_size = 0;
        status = query(gop, pm->Mode, &info_size, &mi);
        if (EFI_ERROR(status) || !mi || !uefi_ptr_ok((UINTN)mi)) {
            return FALSE;
        }
    } else {
        return FALSE;
    }

    /* BltOnly modes have no linear framebuffer (FrameBufferBase is not valid). */
    if (mi->PixelFormat > GOP_PIXEL_BIT_MASK || pm->FrameBufferBase == 0) {
        return FALSE;
    }

    UINT32 bytespp = 4;
    if (mi->PixelFormat == GOP_PIXEL_BGR_8BIT_PER_CHANNEL) {
        info->red_pos = 16;
        info->green_pos = 8;
        info->blue_pos = 0;
        info->red_size = info->green_size = info->blue_size = 8;
    } else if (mi->PixelFormat == GOP_PIXEL_RGB_8BIT_PER_CHANNEL) {
        info->red_pos = 0;
        info->green_pos = 8;
        info->blue_pos = 16;
        info->red_size = info->green_size = info->blue_size = 8;
    } else {
        UINT32 all = mi->PixelInformation[0] | mi->PixelInformation[1] |
                     mi->PixelInformation[2] | mi->PixelInformation[3];
        UINT32 bits = 32;
        if (mi->PixelInformation[0] == 0 || mi->PixelInformation[1] == 0 ||
            mi->PixelInformation[2] == 0) {
            return FALSE;
        }
        while (bits > 0 && (all & (1u << (bits - 1))) == 0) {
            bits--;
        }
        bytespp = (bits + 7u) / 8u;
        info->red_pos = mask_shift(mi->PixelInformation[0]);
        info->red_size = mask_width(mi->PixelInformation[0]);
        info->green_pos = mask_shift(mi->PixelInformation[1]);
        info->green_size = mask_width(mi->PixelInformation[1]);
        info->blue_pos = mask_shift(mi->PixelInformation[2]);
        info->blue_size = mask_width(mi->PixelInformation[2]);
    }

    /* Caller zeroed *info and owns info->magic (set even without GOP). */
    info->framebuffer_addr = pm->FrameBufferBase;
    info->framebuffer_width = mi->HorizontalResolution;
    info->framebuffer_height = mi->VerticalResolution;
    info->framebuffer_pitch = mi->PixelsPerScanLine * bytespp;
    if (info->framebuffer_pitch == 0) {
        info->framebuffer_pitch = info->framebuffer_width * bytespp;
    }
    info->framebuffer_bpp = (UINT8)(bytespp * 8u);
    info->framebuffer_type = 1;
    return TRUE;
}

static UINT64 read_cr3_val(void) {
    UINT64 v;
    __asm__ volatile("mov %%cr3, %0" : "=r"(v));
    return v;
}

static void fill_boot_info_firmware_input(EFI_SYSTEM_TABLE* st, PopcornUefiBootInfo* info) {
    EFI_BOOT_SERVICES_PARTIAL* bs;
    void* input = NULL;

    if (!st || !info || !st->BootServices) {
        return;
    }
    bs = st->BootServices;

    if (!EFI_ERROR(bs->LocateProtocol((EFI_GUID*)&gEfiSimpleTextInputExProtocolGuid, NULL, &input)) &&
        input) {
        info->conin = (UINT64)(UINTN)input;
        info->flags |= POPCORN_UEFI_FLAG_FIRMWARE_KBD | POPCORN_UEFI_FLAG_INPUT_EX;
    } else if (st->ConIn) {
        info->conin = (UINT64)(UINTN)st->ConIn;
        info->flags |= POPCORN_UEFI_FLAG_FIRMWARE_KBD;
    } else {
        return;
    }

    info->boot_services = (UINT64)(UINTN)bs;
    info->firmware_cr3 = read_cr3_val();
}

static EFI_STATUS read_file_from_root(EFI_BOOT_SERVICES_PARTIAL* bs, EFI_FILE_PROTOCOL* root,
    void** out_buf, UINTN* out_size) {
    static const CHAR16* paths[] = {
        (const CHAR16*)L"\\boot\\kernel",
        (const CHAR16*)L"\\BOOT\\KERNEL",
        (const CHAR16*)L"\\kernel",
        (const CHAR16*)L"\\KERNEL",
        (const CHAR16*)L"\\EFI\\BOOT\\kernel",
        (const CHAR16*)L"\\EFI\\BOOT\\KERNEL",
    };

    EFI_FILE_PROTOCOL* file = NULL;
    EFI_STATUS status = EFI_NOT_FOUND;
    for (UINTN i = 0; i < 6; i++) {
        status = root->Open(root, &file, (CHAR16_P)paths[i], 1, 0);
        if (!EFI_ERROR(status) && file) {
            break;
        }
        file = NULL;
    }
    if (!file) {
        root->Close(root);
        return EFI_NOT_FOUND;
    }

    UINT8 chunk[512];
    UINTN chunk_read = sizeof(chunk);
    void* buf = NULL;
    UINTN total = 0;

    for (;;) {
        chunk_read = sizeof(chunk);
        status = file->Read(file, &chunk_read, chunk);
        if (EFI_ERROR(status)) {
            break;
        }
        if (chunk_read == 0) {
            break;
        }
        if (total + chunk_read > MAX_KERNEL_FILE_BYTES) {
            status = EFI_LOAD_ERROR;
            break;
        }
        void* nb = NULL;
        UINTN new_total = total + chunk_read;
        status = bs->AllocatePool(EFI_LOADER_DATA, new_total, &nb);
        if (EFI_ERROR(status)) {
            break;
        }
        if (total > 0) {
            memcpy_local(nb, buf, total);
            bs->FreePool(buf);
        }
        memcpy_local((UINT8*)nb + total, chunk, chunk_read);
        buf = nb;
        total = new_total;
    }

    file->Close(file);
    root->Close(root);

    if (EFI_ERROR(status) || total == 0 || !buf) {
        if (buf) {
            bs->FreePool(buf);
        }
        return EFI_LOAD_ERROR;
    }

    *out_buf = buf;
    *out_size = total;
    return EFI_SUCCESS;
}

static EFI_STATUS read_kernel_try_volume(EFI_BOOT_SERVICES_PARTIAL* bs, EFI_HANDLE vol,
    void** out_buf, UINTN* out_size) {
    EFI_SIMPLE_FILE_SYSTEM_PROTOCOL* fs = NULL;
    EFI_STATUS status = bs->HandleProtocol(vol, &gEfiSimpleFileSystemGuid, (void**)&fs);
    if (EFI_ERROR(status) || !fs) {
        return status;
    }
    EFI_FILE_PROTOCOL* root = NULL;
    status = fs->OpenVolume(fs, &root);
    if (EFI_ERROR(status) || !root) {
        return status;
    }
    return read_file_from_root(bs, root, out_buf, out_size);
}

static int kernel_is_auto_install(const void* buf, UINTN size) {
    /* Split so a safe kernel that embeds this detector is not flagged. */
    static const char a[] = "rust_test_disk";
    static const char b[] = "_install";
    const unsigned char* p = (const unsigned char*)buf;
    const UINTN na = sizeof(a) - 1;
    const UINTN nb = sizeof(b) - 1;
    const UINTN n = na + nb;
    if (!p || size < n) {
        return 0;
    }
    for (UINTN i = 0; i + n <= size; i++) {
        UINTN j;
        for (j = 0; j < na; j++) {
            if (p[i + j] != (unsigned char)a[j]) {
                break;
            }
        }
        if (j != na) {
            continue;
        }
        for (j = 0; j < nb; j++) {
            if (p[i + na + j] != (unsigned char)b[j]) {
                break;
            }
        }
        if (j == nb) {
            return 1;
        }
    }
    return 0;
}

#ifdef POPCORN_TEST_INSTALL
/* scripts refuse to put a loader carrying this marker into a flashable image. */
__attribute__((used)) static const char popcorn_unsafe_loader_marker[] =
    "POPCORN-UNSAFE-TEST-INSTALL-LOADER";

/* CPUID.1:ECX[31]: set by hypervisors, clear on bare metal. */
static int running_under_hypervisor(void) {
    /* Volatile read keeps the marker from being discarded at link time. */
    const volatile char* marker = popcorn_unsafe_loader_marker;
    (void)marker[0];
    UINT32 a = 1, b, c = 0, d;
    __asm__ volatile("cpuid" : "+a"(a), "=b"(b), "+c"(c), "=d"(d));
    (void)b;
    (void)d;
    return (c & (1u << 31)) != 0;
}
#endif

typedef struct {
    UINT32 MediaId;
    BOOLEAN RemovableMedia;
    BOOLEAN MediaPresent;
    BOOLEAN LogicalPartition;
    BOOLEAN ReadOnly;
    BOOLEAN WriteCaching;
    UINT32 BlockSize;
    UINT32 IoAlign;
    UINT64 LastBlock;
} EFI_BLOCK_IO_MEDIA_PARTIAL;

typedef struct _EFI_BLOCK_IO_PROTOCOL EFI_BLOCK_IO_PROTOCOL;
typedef EFI_STATUS (EFIAPI *EFI_BLOCK_READ)(EFI_BLOCK_IO_PROTOCOL* This, UINT32 MediaId,
                                            UINT64 Lba, UINTN BufferSize, void* Buffer);
struct _EFI_BLOCK_IO_PROTOCOL {
    UINT64 Revision;
    EFI_BLOCK_IO_MEDIA_PARTIAL* Media;
    void* Reset;
    EFI_BLOCK_READ ReadBlocks;
};

/* Record the boot volume's identity for the kernel: the BPB serial and total
 * sectors of the FAT volume that holds BOOTX64.EFI. Best effort: on any
 * failure the fields stay zero and the kernel falls back to its own guess. */
static void fill_boot_volume_info(EFI_HANDLE image, EFI_SYSTEM_TABLE* st, PopcornUefiBootInfo* info) {
    static UINT8 sec[4096] __attribute__((aligned(4096)));
    EFI_BOOT_SERVICES_PARTIAL* bs = st->BootServices;
    EFI_LOADED_IMAGE_PROTOCOL* loaded = NULL;
    EFI_BLOCK_IO_PROTOCOL* bio = NULL;
    if (EFI_ERROR(bs->HandleProtocol(image, &gEfiLoadedImageGuid, (void**)&loaded)) ||
        !loaded || !loaded->DeviceHandle) {
        return;
    }
    if (EFI_ERROR(bs->HandleProtocol(loaded->DeviceHandle, &gEfiBlockIoProtocolGuid, (void**)&bio)) ||
        !bio || !bio->Media || !bio->Media->MediaPresent) {
        return;
    }
    UINT32 bsz = bio->Media->BlockSize;
    if (bsz < 512 || bsz > sizeof(sec)) {
        return;
    }
    if (EFI_ERROR(bio->ReadBlocks(bio, bio->Media->MediaId, 0, bsz, sec))) {
        return;
    }
    if (sec[510] != 0x55 || sec[511] != 0xAA) {
        return;
    }
    UINT16 spf16 = (UINT16)(sec[22] | (sec[23] << 8));
    UINT16 tot16 = (UINT16)(sec[19] | (sec[20] << 8));
    UINT32 tot32 = (UINT32)sec[32] | ((UINT32)sec[33] << 8) | ((UINT32)sec[34] << 16) | ((UINT32)sec[35] << 24);
    UINTN so = spf16 == 0 ? 67 : 39; /* FAT32 vs FAT12/16 extended BPB */
    info->boot_vol_serial = (UINT32)sec[so] | ((UINT32)sec[so + 1] << 8) |
                            ((UINT32)sec[so + 2] << 16) | ((UINT32)sec[so + 3] << 24);
    info->boot_vol_sectors = tot16 ? tot16 : tot32;
    info->boot_vol_valid = POPCORN_UEFI_BOOT_VOL_VALID;
}

static EFI_STATUS read_kernel_file(EFI_HANDLE image, EFI_SYSTEM_TABLE* st, void** out_buf, UINTN* out_size) {
    EFI_BOOT_SERVICES_PARTIAL* bs = st->BootServices;

    /* ONLY the volume that loaded BOOTX64.EFI. Scanning every FAT disk used to
     * pick leftover Popcorn on NVMe and run that kernel instead of the USB. */
    EFI_LOADED_IMAGE_PROTOCOL* loaded = NULL;
    EFI_STATUS status = bs->HandleProtocol(image, &gEfiLoadedImageGuid, (void**)&loaded);
    if (EFI_ERROR(status) || !loaded || !loaded->DeviceHandle) {
        return EFI_NOT_FOUND;
    }
    status = read_kernel_try_volume(bs, loaded->DeviceHandle, out_buf, out_size);
    if (EFI_ERROR(status)) {
        return status;
    }
    if (kernel_is_auto_install(*out_buf, *out_size)) {
#ifdef POPCORN_TEST_INSTALL
        /* QEMU test-install loader only (target/UNSAFE, never the flashable
         * image): boot the auto-install kernel, but only inside a VM. */
        if (running_under_hypervisor()) {
            print_err(st, L"UNSAFE test loader: booting auto-install kernel (VM only)");
            return EFI_SUCCESS;
        }
#endif
        bs->FreePool(*out_buf);
        *out_buf = NULL;
        *out_size = 0;
        print_err(st, L"REFUSING kernel: auto-install selftest (unsafe image)");
        return EFI_LOAD_ERROR;
    }
    return EFI_SUCCESS;
}

/* Validated ELF program headers, or NULL. */
static Elf64_Phdr* elf_phdrs(void* kernel_buf, UINTN kernel_size, UINT16* count) {
    if (kernel_size < sizeof(Elf64_Ehdr)) {
        return NULL;
    }
    Elf64_Ehdr* eh = (Elf64_Ehdr*)kernel_buf;
    if (eh->e_ident[0] != 0x7F || eh->e_ident[1] != 'E' || eh->e_ident[2] != 'L' || eh->e_ident[3] != 'F') {
        return NULL;
    }
    if (eh->e_machine != 0x3E || eh->e_phoff == 0 || eh->e_phnum == 0 ||
        eh->e_phentsize != sizeof(Elf64_Phdr)) {
        return NULL;
    }
    if (eh->e_phoff > kernel_size ||
        (UINT64)eh->e_phnum * sizeof(Elf64_Phdr) > kernel_size - eh->e_phoff) {
        return NULL;
    }
    *count = eh->e_phnum;
    return (Elf64_Phdr*)((UINT8*)kernel_buf + eh->e_phoff);
}

/*
 * Whole physical span [lo, hi) of every PT_LOAD, page-aligned. Segments share
 * pages (link.ld packs .rodata right after .text), so per-segment
 * AllocatePages(ADDRESS) used to fail on the shared page and leave the rest of
 * that segment free in the firmware map.
 */
static BOOLEAN elf_load_span(void* kernel_buf, UINTN kernel_size, UINT64* lo_out, UINT64* hi_out) {
    UINT16 n = 0;
    Elf64_Phdr* ph = elf_phdrs(kernel_buf, kernel_size, &n);
    UINT64 lo = ~0ULL;
    UINT64 hi = 0;
    if (!ph) {
        return FALSE;
    }
    for (UINT16 i = 0; i < n; i++) {
        if (ph[i].p_type != PT_LOAD || ph[i].p_memsz == 0) {
            continue;
        }
        if (ph[i].p_filesz > ph[i].p_memsz || ph[i].p_paddr + ph[i].p_memsz < ph[i].p_paddr) {
            return FALSE;
        }
        if (ph[i].p_paddr < lo) {
            lo = ph[i].p_paddr;
        }
        if (ph[i].p_paddr + ph[i].p_memsz > hi) {
            hi = ph[i].p_paddr + ph[i].p_memsz;
        }
    }
    if (hi == 0) {
        return FALSE;
    }
    lo &= ~0xFFFULL;
    hi = (hi + 0xFFFULL) & ~0xFFFULL;
    if (lo < KERNEL_SPAN_MIN_PHYS || hi - lo > KERNEL_SPAN_MAX_BYTES) {
        return FALSE;
    }
    *lo_out = lo;
    *hi_out = hi;
    return TRUE;
}

static BOOLEAN phys_overlaps(UINT64 a, UINT64 alen, UINT64 b, UINT64 blen) {
    return a < b + blen && b < a + alen;
}

/*
 * Own the kernel's physical span before copying into it, so nothing the
 * firmware allocates before ExitBootServices (GOP SetMode, the memory map
 * buffer, timer callbacks) can land on the kernel. kernel_buf came from
 * AllocatePool before the span was known; if it sits inside the span, move it
 * out first. *buf_pages != 0 means *kernel_buf is now a page allocation.
 */
static BOOLEAN reserve_kernel_span(EFI_BOOT_SERVICES_PARTIAL* bs, void** kernel_buf, UINTN kernel_size,
    UINTN* buf_pages, UINT64 lo, UINT64 hi) {
    UINTN span_pages = (UINTN)((hi - lo) / 4096ULL);
    EFI_PHYSICAL_ADDRESS addr = lo;
    EFI_PHYSICAL_ADDRESS blockers[8];
    UINTN nblockers = 0;
    UINTN pages = (kernel_size + 4095u) / 4096u;
    EFI_PHYSICAL_ADDRESS moved = 0;

    if (!EFI_ERROR(bs->AllocatePages(EFI_ALLOCATE_ADDRESS, EFI_LOADER_DATA, span_pages, &addr))) {
        return TRUE;
    }
    if (!phys_overlaps((UINT64)(UINTN)*kernel_buf, kernel_size, lo, hi - lo)) {
        return FALSE; /* firmware owns part of the span: copying would corrupt it */
    }

    /* Relocate the file image; hold any page run that lands in the span. */
    while (nblockers < 8) {
        EFI_PHYSICAL_ADDRESS cand = 0;
        if (EFI_ERROR(bs->AllocatePages(EFI_ALLOCATE_ANY_PAGES, EFI_LOADER_DATA, pages, &cand))) {
            break;
        }
        if (!phys_overlaps(cand, (UINT64)pages * 4096ULL, lo, hi - lo)) {
            moved = cand;
            break;
        }
        blockers[nblockers++] = cand;
    }
    for (UINTN i = 0; i < nblockers; i++) {
        bs->FreePages(blockers[i], pages);
    }
    if (moved == 0) {
        return FALSE;
    }
    memcpy_local((void*)(UINTN)moved, *kernel_buf, kernel_size);
    bs->FreePool(*kernel_buf);
    *kernel_buf = (void*)(UINTN)moved;
    *buf_pages = pages;

    addr = lo;
    return !EFI_ERROR(bs->AllocatePages(EFI_ALLOCATE_ADDRESS, EFI_LOADER_DATA, span_pages, &addr));
}

static void free_kernel_buf(EFI_BOOT_SERVICES_PARTIAL* bs, void* kernel_buf, UINTN buf_pages) {
    if (!kernel_buf) {
        return;
    }
    if (buf_pages) {
        bs->FreePages((EFI_PHYSICAL_ADDRESS)(UINTN)kernel_buf, buf_pages);
    } else {
        bs->FreePool(kernel_buf);
    }
}

/* Copy PT_LOAD segments into the span reserve_kernel_span() already owns. */
static BOOLEAN load_elf_segments(void* kernel_buf, UINTN kernel_size) {
    UINT16 n = 0;
    Elf64_Phdr* ph = elf_phdrs(kernel_buf, kernel_size, &n);
    if (!ph) {
        return FALSE;
    }
    for (UINT16 i = 0; i < n; i++) {
        if (ph[i].p_type != PT_LOAD) {
            continue;
        }
        if (ph[i].p_memsz == 0) {
            continue;
        }

        if (ph[i].p_filesz > 0) {
            if (ph[i].p_offset > kernel_size || ph[i].p_filesz > kernel_size - ph[i].p_offset) {
                return FALSE;
            }
            memcpy_local((void*)(UINTN)ph[i].p_paddr, (UINT8*)kernel_buf + ph[i].p_offset, (UINTN)ph[i].p_filesz);
        }
        if (ph[i].p_memsz > ph[i].p_filesz) {
            UINTN z = (UINTN)(ph[i].p_memsz - ph[i].p_filesz);
            memset_local((void*)(UINTN)(ph[i].p_paddr + ph[i].p_filesz), 0, z);
        }
    }
    return TRUE;
}

static int str_eq(const char* a, const char* b) {
    while (*a && *b) {
        if (*a != *b) {
            return 0;
        }
        a++;
        b++;
    }
    return *a == *b;
}

static UINT64 find_elf_symbol(void* kernel_buf, UINTN kernel_size, const char* want) {
    if (kernel_size < sizeof(Elf64_Ehdr) + sizeof(Elf64_Shdr)) {
        return 0;
    }
    Elf64_Ehdr* eh = (Elf64_Ehdr*)kernel_buf;
    if (eh->e_shoff == 0 || eh->e_shentsize < sizeof(Elf64_Shdr) || eh->e_shstrndx == 0) {
        return 0;
    }
    Elf64_Shdr* sh = (Elf64_Shdr*)((UINT8*)kernel_buf + eh->e_shoff);
    Elf64_Shdr* shstr = &sh[eh->e_shstrndx];
    if (shstr->sh_offset + shstr->sh_size > kernel_size) {
        return 0;
    }

    for (UINT16 i = 0; i < eh->e_shnum; i++) {
        if (sh[i].sh_type != 2) {
            continue;
        }
        if (sh[i].sh_offset + sh[i].sh_size > kernel_size || sh[i].sh_link >= eh->e_shnum) {
            continue;
        }
        Elf64_Shdr* strsh = &sh[sh[i].sh_link];
        if (strsh->sh_offset + strsh->sh_size > kernel_size) {
            continue;
        }
        const char* strtab = (const char*)((UINT8*)kernel_buf + strsh->sh_offset);
        UINTN sym_count = sh[i].sh_size / sizeof(Elf64_Sym);
        Elf64_Sym* syms = (Elf64_Sym*)((UINT8*)kernel_buf + sh[i].sh_offset);
        for (UINTN s = 0; s < sym_count; s++) {
            if (syms[s].st_name == 0 || syms[s].st_name >= strsh->sh_size) {
                continue;
            }
            const char* name = strtab + syms[s].st_name;
            if (str_eq(name, want)) {
                return syms[s].st_value;
            }
        }
    }
    return 0;
}

static void __attribute__((noreturn)) jump_to_kernel(UINT64 entry) {
    __asm__ volatile("cli");
    __asm__ volatile("jmp *%0" : : "r"(entry) : "memory");
    __builtin_unreachable();
}

#define EFI_CONVENTIONAL_MEMORY 7u
#define MB2_TAG_END               0u
#define MB2_TAG_BOOT_LOADER_NAME  2u
#define MB2_TAG_BASIC_MEMINFO     4u
#define MB2_TAG_MMAP              6u
#define MB2_MEM_AVAILABLE         1u
#define MB2_MEM_RESERVED          2u
#define MB2_MMAP_ENTRY_SIZE       24u

static UINT32 mb2_align(UINT32 n) {
    return (n + 7u) & ~7u;
}

/* MBI bytes besides the mmap entries: header, loader-name, mmap header, meminfo, end. */
#define MB2_FIXED_BYTES (8u + 24u + 16u + 16u + 8u)

/*
 * Build the Multiboot2 MBI in `out` (cap bytes, AllocatePages'd before the
 * ExitBootServices loop so it is LoaderData in the final map and never part of
 * the kernel image). Calls no boot services: it runs between GetMemoryMap and
 * ExitBootServices, including on retries.
 */
static void build_multiboot_mbi(UINT8* out, UINTN cap, EFI_MEMORY_DESCRIPTOR* mmap, UINTN map_size,
    UINTN desc_size, PopcornUefiBootInfo* info) {
    UINT32 total = 8;
    UINT64 mem_upper_kb = 0;
    UINT32 max_entries = (cap > MB2_FIXED_BYTES)
                             ? (UINT32)((cap - MB2_FIXED_BYTES) / MB2_MMAP_ENTRY_SIZE)
                             : 0;

    memset_local(out, 0, cap);

    {
        const char loader_name[] = "Popcorn UEFI";
        UINT32 name_len = sizeof(loader_name);
        UINT32 tag_sz = mb2_align(8u + name_len);
        UINT32* tag = (UINT32*)(out + total);
        tag[0] = MB2_TAG_BOOT_LOADER_NAME;
        tag[1] = tag_sz;
        for (UINT32 i = 0; i < name_len; i++) {
            out[total + 8u + i] = (UINT8)loader_name[i];
        }
        total += tag_sz;
    }

    UINT32 mmap_count = 0;
    UINTN num_desc = (desc_size > 0) ? (map_size / desc_size) : 0;
    for (UINTN i = 0; i < num_desc; i++) {
        EFI_MEMORY_DESCRIPTOR* d =
            (EFI_MEMORY_DESCRIPTOR*)((UINT8*)mmap + i * desc_size);
        UINT64 len = d->NumberOfPages * 4096ULL;
        if (len == 0) {
            continue;
        }
        mmap_count++;
    }
    if (mmap_count > max_entries) {
        mmap_count = max_entries; /* cap is sized from the map buffer; never expected */
    }

    {
        UINT32 tag_sz = mb2_align(16u + mmap_count * MB2_MMAP_ENTRY_SIZE);
        UINT32* tag = (UINT32*)(out + total);
        tag[0] = MB2_TAG_MMAP;
        tag[1] = tag_sz;
        tag[2] = MB2_MMAP_ENTRY_SIZE;
        tag[3] = 0;
        UINT32 entry_off = total + 16u;
        UINT32 written = 0;
        for (UINTN i = 0; i < num_desc && written < mmap_count; i++) {
            EFI_MEMORY_DESCRIPTOR* d =
                (EFI_MEMORY_DESCRIPTOR*)((UINT8*)mmap + i * desc_size);
            UINT64 len = d->NumberOfPages * 4096ULL;
            if (len == 0) {
                continue;
            }
            UINT64 addr = d->PhysicalStart;
            UINT32 mb2_type = MB2_MEM_RESERVED;
            if (d->Type == EFI_CONVENTIONAL_MEMORY) {
                mb2_type = MB2_MEM_AVAILABLE;
                if (addr >= 0x100000ULL) {
                    /* Sum in 64-bit KB — UINT32 overflow used to wrap toward nonsense totals. */
                    mem_upper_kb += len / 1024ULL;
                }
            }
            UINT8* e = out + entry_off + written * MB2_MMAP_ENTRY_SIZE;
            *(UINT64*)(e + 0) = addr;
            *(UINT64*)(e + 8) = len;
            *(UINT32*)(e + 16) = mb2_type;
            *(UINT32*)(e + 20) = 0;
            written++;
        }
        total += tag_sz;
    }

    {
        UINT32 tag_sz = mb2_align(sizeof(UINT32) * 4u);
        UINT32* tag = (UINT32*)(out + total);
        tag[0] = MB2_TAG_BASIC_MEMINFO;
        tag[1] = tag_sz;
        tag[2] = 640;
        /* Multiboot basic meminfo field is 32-bit KB; clamp (mmap tag has the truth). */
        tag[3] = (mem_upper_kb > 0xFFFFFFFFULL) ? 0xFFFFFFFFu : (UINT32)mem_upper_kb;
        total += tag_sz;
    }

    {
        UINT32 tag_sz = 8;
        UINT32* tag = (UINT32*)(out + total);
        tag[0] = MB2_TAG_END;
        tag[1] = tag_sz;
        total += tag_sz;
    }

    *(UINT32*)(out + 0) = total;

    info->available_ram_bytes = mem_upper_kb * 1024ULL;
}

/*
 * Allocate the memory-map buffer and the MBI once, with slack, then loop on
 * GetMemoryMap + ExitBootServices only. After the first ExitBootServices call
 * the firmware may have torn down parts of boot services, so a retry must not
 * call AllocatePool/FreePool (or print). *ebs_attempted tells the caller
 * whether ConOut is still safe to use.
 */
static BOOLEAN exit_boot_services(EFI_HANDLE image, EFI_SYSTEM_TABLE* st, PopcornUefiBootInfo* info,
    BOOLEAN* ebs_attempted) {
    EFI_BOOT_SERVICES_PARTIAL* bs = st->BootServices;
    EFI_STATUS status;
    UINTN map_size = 0;
    UINTN map_cap = 0;
    UINTN map_key = 0;
    UINTN desc_size = 0;
    UINT32 desc_ver = 0;
    EFI_MEMORY_DESCRIPTOR* mmap = NULL;
    EFI_PHYSICAL_ADDRESS mbi = MBI_MAX_PHYS;
    UINTN mbi_pages;
    UINTN max_desc;

    *ebs_attempted = FALSE;

    status = bs->GetMemoryMap(&map_size, NULL, &map_key, &desc_size, &desc_ver);
    if (status != EFI_BUFFER_TOO_SMALL || desc_size < sizeof(EFI_MEMORY_DESCRIPTOR)) {
        return FALSE;
    }
    /* Slack for our own two allocations below plus firmware churn between calls. */
    map_cap = map_size + EFI_MMAP_PAGES * 4096;
    status = bs->AllocatePool(EFI_LOADER_DATA, map_cap, (void**)&mmap);
    if (EFI_ERROR(status) || !mmap) {
        return FALSE;
    }

    max_desc = map_cap / desc_size;
    mbi_pages = (MB2_FIXED_BYTES + max_desc * MB2_MMAP_ENTRY_SIZE + 4095u) / 4096u;
    if (mbi_pages < MBI_MIN_PAGES) {
        mbi_pages = MBI_MIN_PAGES;
    }
    status = bs->AllocatePages(EFI_ALLOCATE_MAX_ADDRESS, EFI_LOADER_DATA, mbi_pages, &mbi);
    if (EFI_ERROR(status) || mbi == 0) {
        bs->FreePool(mmap);
        return FALSE;
    }
    info->mbi_phys = (UINT64)mbi;

    for (UINTN attempt = 0; attempt < 8; attempt++) {
        map_size = map_cap;
        status = bs->GetMemoryMap(&map_size, mmap, &map_key, &desc_size, &desc_ver);
        if (EFI_ERROR(status)) {
            break;
        }
        build_multiboot_mbi((UINT8*)(UINTN)mbi, mbi_pages * 4096u, mmap, map_size, desc_size, info);
        *ebs_attempted = TRUE;
        status = bs->ExitBootServices(image, map_key);
        if (!EFI_ERROR(status)) {
            return TRUE;
        }
        if (status != EFI_INVALID_PARAMETER) {
            break;
        }
    }
    if (!*ebs_attempted) {
        bs->FreePages(mbi, mbi_pages);
        bs->FreePool(mmap);
        info->mbi_phys = 0;
    }
    return FALSE;
}

/* MSVC ABI stack probe; freestanding UEFI has no CRT. */
void __chkstk(void) {}

EFI_STATUS EFIAPI efi_main(EFI_HANDLE image, EFI_SYSTEM_TABLE* system_table) {
    if (!system_table || !system_table->BootServices) {
        return EFI_LOAD_ERROR;
    }

    if (system_table->ConOut) {
        system_table->ConOut->ClearScreen(system_table->ConOut);
    }
    print(system_table, L"Popcorn UEFI Loader v10\r\n");
#ifdef POPCORN_TEST_INSTALL
    print(system_table, L"UNSAFE QEMU test-install loader: never flash this image\r\n");
#endif

    void* kernel_buf = NULL;
    UINTN kernel_size = 0;
    UINTN kernel_buf_pages = 0; /* 0: kernel_buf is pool memory */
    UINT64 span_lo = 0;
    UINT64 span_hi = 0;
    BOOLEAN ebs_attempted = FALSE;
    EFI_STATUS status = read_kernel_file(image, system_table, &kernel_buf, &kernel_size);
    if (EFI_ERROR(status)) {
        print_err(system_table, L"Could not read \\boot\\kernel");
        return status;
    }

    UINT64 entry = find_elf_symbol(kernel_buf, kernel_size, "uefi_start");
    if (entry < 0x100000ULL || entry >= 0x300000ULL) {
        entry = 0;
    }
    /* Always write GOP handoff at the fixed boot .bss slot (see kernel.asm / uefi_boot.h). */
    UINT64 handoff = POPCORN_UEFI_HANDOFF_PHYS;
    EFI_BOOT_SERVICES_PARTIAL* bs = system_table->BootServices;
    if (entry == 0) {
        bs->FreePool(kernel_buf);
        print_err(system_table, L"uefi_start symbol missing");
        return EFI_LOAD_ERROR;
    }

    if (!elf_load_span(kernel_buf, kernel_size, &span_lo, &span_hi) ||
        handoff < span_lo || handoff + sizeof(PopcornUefiBootInfo) > span_hi) {
        bs->FreePool(kernel_buf);
        print_err(system_table, L"ELF load failed (bad program headers)");
        return EFI_LOAD_ERROR;
    }
    if (!reserve_kernel_span(bs, &kernel_buf, kernel_size, &kernel_buf_pages, span_lo, span_hi)) {
        free_kernel_buf(bs, kernel_buf, kernel_buf_pages);
        print_err(system_table, L"Kernel physical range is in use by firmware");
        return EFI_LOAD_ERROR;
    }
    if (!load_elf_segments(kernel_buf, kernel_size)) {
        free_kernel_buf(bs, kernel_buf, kernel_buf_pages);
        print_err(system_table, L"ELF load failed");
        return EFI_LOAD_ERROR;
    }
    free_kernel_buf(bs, kernel_buf, kernel_buf_pages);

    {
        PopcornUefiBootInfo* info = (PopcornUefiBootInfo*)(UINTN)handoff;
        memset_local(info, 0, sizeof(PopcornUefiBootInfo));
        /* Magic marks a Popcorn UEFI boot even without GOP: the kernel needs it
         * to find the MBI and pick the UEFI timer/memory paths. framebuffer_addr
         * stays 0 when there is no usable framebuffer. */
        info->magic = POPCORN_UEFI_MAGIC;
        if (!fill_boot_info_from_gop(system_table, info)) {
            print(system_table, L"No GOP framebuffer available.\r\n");
        }
        fill_boot_info_firmware_input(system_table, info);
        fill_boot_volume_info(image, system_table, info);
    }

    print(system_table, L"Starting Popcorn...\r\n");

    /*
     * Always ExitBootServices before entering the kernel.
     * Keeping Boot Services up for firmware ConIn caused PMM/allocator corruption
     * and firmware-initiated resets (~6–8s after the shell appeared) on QEMU and
     * ThinkPad-class hardware. PS/2 polling handles keyboard after EBS.
     */
    if (!exit_boot_services(image, system_table, (PopcornUefiBootInfo*)(UINTN)handoff, &ebs_attempted)) {
        if (!ebs_attempted) {
            print_err(system_table, L"ExitBootServices failed");
        }
        return EFI_LOAD_ERROR;
    }

    {
        PopcornUefiBootInfo* info = (PopcornUefiBootInfo*)(UINTN)handoff;
        info->flags &= ~(POPCORN_UEFI_FLAG_FIRMWARE_KBD | POPCORN_UEFI_FLAG_INPUT_EX);
        info->conin = 0;
        info->boot_services = 0;
        info->firmware_cr3 = 0;
    }

    jump_to_kernel(entry);
}
