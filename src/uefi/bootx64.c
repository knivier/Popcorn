/*
 * Popcorn native UEFI loader (BOOTX64.EFI).
 * Loads /boot/kernel ELF segments and jumps to kernel entry (0x100030).
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
#define EFI_ALLOCATE_ADDRESS                 0x00000002
#define EFI_MEMORY_WB                        0x00000006
#define EFI_MMAP_PAGES                       8
#define PT_LOAD                              1
#define GOP_PIXEL_BGR_8BIT_PER_CHANNEL       6
#define GOP_PIXEL_RGB_8BIT_PER_CHANNEL       1
#define MAX_KERNEL_FILE_BYTES                (16U * 1024U * 1024U)

typedef struct {
    UINT32 Data1;
    UINT16 Data2;
    UINT16 Data3;
    UINT8  Data4[8];
} EFI_GUID;

static const EFI_GUID gEfiLoadedImageGuid =
    {0x5B1B31A1, 0x9562, 0x11d2, {0x8E, 0x3F, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B}};
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

typedef struct {
    EFI_TABLE_HEADER Hdr;
    void* RaiseTPL;
    void* RestoreTPL;
    EFI_ALLOCATE_PAGES AllocatePages;
    void* FreePages;
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

static BOOLEAN uefi_ptr_ok(UINTN p) {
    return p >= 0x1000ULL && p < 0xFFFFFFFFFFFFULL;
}

static BOOLEAN fill_boot_info_from_gop(EFI_SYSTEM_TABLE* st, PopcornUefiBootInfo* info) {
    EFI_BOOT_SERVICES_PARTIAL* bs = st->BootServices;
    EFI_GRAPHICS_OUTPUT_PROTOCOL* gop = NULL;
    EFI_STATUS status = bs->LocateProtocol(&gEfiGraphicsOutputProtocolGuid, NULL, (void**)&gop);
    if (EFI_ERROR(status) || !gop || !uefi_ptr_ok((UINTN)gop)) {
        return FALSE;
    }

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

    memset_local(info, 0, sizeof(PopcornUefiBootInfo));
    info->magic = POPCORN_UEFI_MAGIC;
    info->framebuffer_addr = pm->FrameBufferBase;
    info->framebuffer_width = mi->HorizontalResolution;
    info->framebuffer_height = mi->VerticalResolution;
    info->framebuffer_pitch = mi->PixelsPerScanLine * 4;
    if (info->framebuffer_pitch == 0) {
        info->framebuffer_pitch = info->framebuffer_width * 4;
    }
    info->framebuffer_bpp = 32;
    info->framebuffer_type = 1;
    if (mi->PixelFormat == GOP_PIXEL_BGR_8BIT_PER_CHANNEL) {
        info->red_pos = 16;
        info->green_pos = 8;
        info->blue_pos = 0;
        info->red_size = info->green_size = info->blue_size = 8;
    } else {
        info->red_pos = 0;
        info->green_pos = 8;
        info->blue_pos = 16;
        info->red_size = info->green_size = info->blue_size = 8;
    }
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
        (const CHAR16*)L"\\kernel",
        (const CHAR16*)L"\\EFI\\BOOT\\kernel",
    };

    EFI_FILE_PROTOCOL* file = NULL;
    EFI_STATUS status = EFI_NOT_FOUND;
    for (UINTN i = 0; i < 3; i++) {
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

static EFI_STATUS read_kernel_file(EFI_HANDLE image, EFI_SYSTEM_TABLE* st, void** out_buf, UINTN* out_size) {
    EFI_BOOT_SERVICES_PARTIAL* bs = st->BootServices;

    EFI_LOADED_IMAGE_PROTOCOL* loaded = NULL;
    EFI_STATUS status = bs->HandleProtocol(image, &gEfiLoadedImageGuid, (void**)&loaded);
    if (!EFI_ERROR(status) && loaded && loaded->DeviceHandle) {
        status = read_kernel_try_volume(bs, loaded->DeviceHandle, out_buf, out_size);
        if (!EFI_ERROR(status)) {
            return EFI_SUCCESS;
        }
    }

    UINTN buf_size = 0;
    status = bs->LocateHandle(EFI_LOCATE_BY_PROTOCOL, (EFI_GUID*)&gEfiSimpleFileSystemGuid, NULL,
        &buf_size, NULL);
    if (status != EFI_BUFFER_TOO_SMALL || buf_size == 0) {
        return EFI_NOT_FOUND;
    }

    EFI_HANDLE* handles = NULL;
    status = bs->AllocatePool(EFI_LOADER_DATA, buf_size, (void**)&handles);
    if (EFI_ERROR(status)) {
        return status;
    }

    status = bs->LocateHandle(EFI_LOCATE_BY_PROTOCOL, (EFI_GUID*)&gEfiSimpleFileSystemGuid, NULL,
        &buf_size, handles);
    if (EFI_ERROR(status)) {
        bs->FreePool(handles);
        return status;
    }

    UINTN count = buf_size / sizeof(EFI_HANDLE);
    if (count > 8) {
        count = 8;
    }
    for (UINTN i = 0; i < count; i++) {
        status = read_kernel_try_volume(bs, handles[i], out_buf, out_size);
        if (!EFI_ERROR(status)) {
            bs->FreePool(handles);
            return EFI_SUCCESS;
        }
    }

    bs->FreePool(handles);
    return EFI_NOT_FOUND;
}

static BOOLEAN load_elf_segments(void* kernel_buf, UINTN kernel_size, EFI_BOOT_SERVICES_PARTIAL* bs) {
    if (kernel_size < sizeof(Elf64_Ehdr)) {
        return FALSE;
    }
    Elf64_Ehdr* eh = (Elf64_Ehdr*)kernel_buf;
    if (eh->e_ident[0] != 0x7F || eh->e_ident[1] != 'E' || eh->e_ident[2] != 'L' || eh->e_ident[3] != 'F') {
        return FALSE;
    }
    if (eh->e_machine != 0x3E || eh->e_phoff == 0 || eh->e_phnum == 0) {
        return FALSE;
    }

    Elf64_Phdr* ph = (Elf64_Phdr*)((UINT8*)kernel_buf + eh->e_phoff);
    for (UINT16 i = 0; i < eh->e_phnum; i++) {
        if (ph[i].p_type != PT_LOAD) {
            continue;
        }
        if (ph[i].p_memsz == 0) {
            continue;
        }
        /* Reserve the kernel's fixed physical pages best-effort. Firmware may
         * refuse a fixed address if it already owns it; the low conventional
         * RAM the kernel targets is writable before ExitBootServices either way. */
        UINTN pages = (UINTN)((ph[i].p_memsz + 4095) / 4096);
        EFI_PHYSICAL_ADDRESS dest = ph[i].p_paddr & ~0xFFFULL;
        bs->AllocatePages(EFI_ALLOCATE_ADDRESS, EFI_LOADER_DATA, pages, &dest);

        if (ph[i].p_filesz > 0) {
            if (ph[i].p_offset + ph[i].p_filesz > kernel_size) {
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

static BOOLEAN exit_boot_services(EFI_HANDLE image, EFI_SYSTEM_TABLE* st) {
    EFI_BOOT_SERVICES_PARTIAL* bs = st->BootServices;
    EFI_STATUS status;
    UINTN map_size = 0;
    UINTN map_key = 0;
    UINTN desc_size = 0;
    UINT32 desc_ver = 0;
    EFI_MEMORY_DESCRIPTOR* mmap = NULL;

    for (UINTN attempt = 0; attempt < 8; attempt++) {
        map_size = 0;
        status = bs->GetMemoryMap(&map_size, NULL, &map_key, &desc_size, &desc_ver);
        if (status != EFI_BUFFER_TOO_SMALL) {
            return FALSE;
        }
        map_size += 8 * 4096;
        if (mmap) {
            bs->FreePool(mmap);
            mmap = NULL;
        }
        status = bs->AllocatePool(EFI_LOADER_DATA, map_size, (void**)&mmap);
        if (EFI_ERROR(status)) {
            return FALSE;
        }
        status = bs->GetMemoryMap(&map_size, mmap, &map_key, &desc_size, &desc_ver);
        if (EFI_ERROR(status)) {
            bs->FreePool(mmap);
            return FALSE;
        }
        status = bs->ExitBootServices(image, map_key);
        if (!EFI_ERROR(status)) {
            return TRUE;
        }
        if (status != EFI_INVALID_PARAMETER) {
            bs->FreePool(mmap);
            return FALSE;
        }
    }
    if (mmap) {
        bs->FreePool(mmap);
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
    print(system_table, L"Popcorn UEFI Loader v8\r\n");

    void* kernel_buf = NULL;
    UINTN kernel_size = 0;
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
    if (entry == 0) {
        system_table->BootServices->FreePool(kernel_buf);
        print_err(system_table, L"uefi_start symbol missing");
        return EFI_LOAD_ERROR;
    }

    EFI_BOOT_SERVICES_PARTIAL* bs = system_table->BootServices;
    if (!load_elf_segments(kernel_buf, kernel_size, bs)) {
        system_table->BootServices->FreePool(kernel_buf);
        print_err(system_table, L"ELF load failed");
        return EFI_LOAD_ERROR;
    }
    bs->FreePool(kernel_buf);

    {
        PopcornUefiBootInfo* info = (PopcornUefiBootInfo*)(UINTN)handoff;
        memset_local(info, 0, sizeof(PopcornUefiBootInfo));
        if (!fill_boot_info_from_gop(system_table, info)) {
            print(system_table, L"No GOP framebuffer available.\r\n");
        }
        fill_boot_info_firmware_input(system_table, info);
    }

    print(system_table, L"Starting Popcorn...\r\n");

    {
        PopcornUefiBootInfo* info = (PopcornUefiBootInfo*)(UINTN)handoff;
        if (info->flags & POPCORN_UEFI_FLAG_FIRMWARE_KBD) {
            print(system_table, L"Keeping UEFI USB keyboard (no ExitBootServices)\r\n");
        } else if (!exit_boot_services(image, system_table)) {
            print_err(system_table, L"ExitBootServices failed");
            return EFI_LOAD_ERROR;
        }
    }

    jump_to_kernel(entry);
}
