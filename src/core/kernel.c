#include "../includes/uefi_input.h"
#include "../includes/console.h"
#include "../includes/dolphin_pop.h"
#include "../includes/timer.h"
#include "../includes/scheduler.h"
#include "../includes/init.h"
#include "../includes/keyboard_map.h"
#include "../includes/kbd.h"
#include "../includes/shell.h"
#include "../includes/phase2_selftest.h"
#include "../includes/utils.h"
#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>

extern unsigned int history_count;
const char* get_current_directory(void);

#define ENTER_KEY_CODE 0x1C
#define BACKSPACE_KEY_CODE 0x0E
#define UP_ARROW_CODE 0x48
#define DOWN_ARROW_CODE 0x50
#define LEFT_ARROW_CODE 0x4B
#define RIGHT_ARROW_CODE 0x4D
#define PAGE_UP_CODE 0x49
#define PAGE_DOWN_CODE 0x51
#define TAB_KEY_CODE 0x0F
#define KEYBOARD_EXT_PREFIX 0xE0

extern void boot_serial_putc(char c);

/* Shared console cursor byte offset (shell history / legacy helpers). */
unsigned int current_loc = 0;
ConsoleState console_state = {0, 0, CONSOLE_FG_COLOR, true, false, 0};

/* Modifier state from the scancode loop; read by Dolphin via keyboard_map.h. */
static bool kbd_shift = false;
static bool kbd_caps = false;
int kbd_shift_active(void) { return kbd_shift ? 1 : 0; }
int kbd_caps_active(void) { return kbd_caps ? 1 : 0; }

void kmain(void) {
    init_boot_screen();
    boot_serial_putc('K');

    // At this point, init_boot_screen() has already:
    // - Initialized memory management
    // - Initialized timer system
    // - Initialized scheduler
    // - Loaded all pop modules
    // - Set up interrupt handlers
    // - Enabled timer interrupts
    
    // Initialize input buffer and history
    /* A typed line must fit one 128-byte history slot (127 chars + NUL). */
#define INPUT_MAX 127u
    char input_buffer[INPUT_MAX + 1] = {0};
    char temp_buffer[INPUT_MAX + 1] = {0};
    unsigned int input_index = 0;
    int history_index = -1;
    /* PS/2 set 1: 0xE0 byte prefixes extended scancode; break = make | 0x80. */
    static bool kbd_expect_e0 = false;

    uint64_t loop_iter = 0;
    uint64_t last_uefi_poll_tick = 0;

    boot_serial_putc('L');
    /*
     * Timer mode is chosen in init_transition_to_console():
     * UEFI → poll-only PIT; GRUB → IRQ0 PIT. Do not force poll here.
     */
    if (!timer_is_poll_mode()) {
        __asm__ volatile("sti" ::: "memory");
    }
    scheduler_end_bootstrap();
    boot_serial_putc('M');
    /* Phase 2 exit criteria: ioctl→device + sleep/wait-queue wake (serial I/B/S/2). */
    phase2_selftest();

#ifdef POPCORN_TEST_PF
    /* CI: force a page fault after IDT is live; expect #PF dump on COM1/debugcon.
     * PML4[1] is unmapped (VA bit 39) on both GRUB and UEFI page tables. */
    *(volatile uint32_t*)(uintptr_t)(1ULL << 39) = 0x50465046u;
#endif

    while (1) {
        unsigned char keycode;
        bool from_queue = false;

        timer_poll();
        if ((loop_iter & 0xFFu) == 0u) {
            console_heartbeat_tick();
        }
        loop_iter++;

        /* Firmware ReadKeyStroke can hang; poll at most ~10 Hz via PIT ticks. */
        if (uefi_input_available()) {
            uint64_t now = timer_get_ticks();
            if (now - last_uefi_poll_tick >= 10u) {
                bool is_scancode = false;
                last_uefi_poll_tick = now;
                if (uefi_input_poll(&keycode, &is_scancode)) {
                    if (!is_scancode) {
                        if (dolphin_is_active()) {
                            /* UEFI delivers Unicode, not scancodes — feed the editor. */
                            char ch = (char)keycode;
                            if (ch == '\r' || ch == '\n') {
                                dolphin_handle_key(0x1C); /* Enter */
                            } else if (ch == 0x08 || ch == 0x7F) {
                                dolphin_handle_key(0x0E); /* Backspace */
                            } else if (ch == 0x1B) {
                                dolphin_handle_key(0x01); /* Esc */
                            } else if (ch >= 32 && ch < 127) {
                                dolphin_insert_char(ch);
                                dolphin_render();
                            }
                            continue;
                        }
                        if (input_index < INPUT_MAX) {
                            console_scroll_to_bottom();
                            input_buffer[input_index++] = (char)keycode;
                            console_set_color(CONSOLE_BG_COLOR | COLOR_WHITE);
                            console_putchar((char)keycode);
                            history_index = -1;
                        }
                        continue;
                    }
                    from_queue = true;
                }
            }
        }

        /* Scancodes via /dev/kbd device ops (Rust drive). */
        if (!from_queue && !kbd_read_scancode(&keycode)) {
            /* UEFI/QEMU: sti+hlt hangs on latent IRQ; polled PIT keeps time alive. */
            if (timer_is_poll_mode()) {
                __asm__ volatile("pause");
            } else {
                __asm__ volatile("sti; hlt" ::: "memory");
            }
            continue;
        }

        if (kbd_expect_e0) {
            kbd_expect_e0 = false;
            if (keycode & 0x80) {
                continue; /* extended key release, e.g. 0xC8 */
            }
            /* extended make: 0x48/50/4B/4D/49/51 — same values as our arrow/page defs */
        } else if (keycode == KEYBOARD_EXT_PREFIX) {
            kbd_expect_e0 = true;
            continue;
        } else if (keycode & 0x80) {
            /* Releases: track Shift; ignore other breaks. */
            uint8_t make = (uint8_t)(keycode & 0x7F);
            if (make == KBD_SCAN_LSHIFT || make == KBD_SCAN_RSHIFT) {
                kbd_shift = false;
            }
            continue;
        } else {
            if (keycode == KBD_SCAN_LSHIFT || keycode == KBD_SCAN_RSHIFT) {
                kbd_shift = true;
                continue;
            }
            if (keycode == KBD_SCAN_CAPS) {
                kbd_caps = !kbd_caps;
                continue;
            }
        }

        if (dolphin_is_active()) {
            dolphin_handle_key(keycode);
            continue;
        }

        if (keycode == ENTER_KEY_CODE) {
            console_scroll_to_bottom();
            input_buffer[input_index] = '\0';
            add_to_history(input_buffer);  // Add to history
            console_newline();
            execute_command(input_buffer);
            input_index = 0;
            history_index = -1;  // Reset history browsing
            memset(input_buffer, 0, sizeof(input_buffer));
            memset(temp_buffer, 0, sizeof(temp_buffer));
            console_set_color(CONSOLE_BG_COLOR | COLOR_WHITE);
            console_newline();
            console_draw_prompt_with_path(get_current_directory());
            console_print_status_bar();
            console_set_color(CONSOLE_BG_COLOR | COLOR_WHITE);
        } else if (keycode == BACKSPACE_KEY_CODE) {
            console_scroll_to_bottom();
            if (input_index > 0) {
                input_index--;
                input_buffer[input_index] = '\0';
                console_backspace();
            }
        } else if (keycode == TAB_KEY_CODE) {
            console_scroll_to_bottom();
            autocomplete_command(input_buffer, &input_index);
        } else if (keycode == UP_ARROW_CODE) {
            /* Terminal scrollback (older lines). */
            console_scroll_up();
        } else if (keycode == DOWN_ARROW_CODE) {
            /* Terminal scrollback (toward live prompt). */
            console_scroll_down();
        } else if (keycode == LEFT_ARROW_CODE) {
            /* Command history: older. */
            console_scroll_to_bottom();
            if (history_count > 0) {
                if (history_index == -1) {
                    strcpy_simple(temp_buffer, input_buffer);
                    history_index = (int)history_count;
                }
                if (history_index > 0) {
                    history_index--;
                    const char *cmd = get_history_command(history_index);
                    if (cmd) {
                        while (input_index > 0) {
                            input_index--;
                            console_backspace();
                        }
                        strcpy_simple(input_buffer, cmd);
                        input_index = strlen_simple(input_buffer);
                        console_print(input_buffer);
                    }
                }
            }
        } else if (keycode == RIGHT_ARROW_CODE) {
            /* Command history: newer / back to draft. */
            console_scroll_to_bottom();
            if (history_index != -1) {
                history_index++;
                while (input_index > 0) {
                    input_index--;
                    console_backspace();
                }
                if (history_index >= (int)history_count) {
                    strcpy_simple(input_buffer, temp_buffer);
                    history_index = -1;
                } else {
                    const char *cmd = get_history_command(history_index);
                    if (cmd) {
                        strcpy_simple(input_buffer, cmd);
                    }
                }
                input_index = strlen_simple(input_buffer);
                console_print(input_buffer);
            }
        } else if (keycode == PAGE_UP_CODE) {
            for (int i = 0; i < 10; i++) {
                console_scroll_up();
            }
        } else if (keycode == PAGE_DOWN_CODE) {
            for (int i = 0; i < 10; i++) {
                console_scroll_down();
            }
        } else if (input_index < INPUT_MAX && keycode < 128) {
            char ch = kbd_scancode_to_char(keycode, kbd_shift, kbd_caps);
            /* Printable ASCII only: no stray control bytes in the command line. */
            if (ch >= ' ' && ch < 127) {
                console_scroll_to_bottom();
                input_buffer[input_index++] = ch;
                console_set_color(CONSOLE_BG_COLOR | COLOR_WHITE);
                console_putchar(ch);
                history_index = -1;
            }
        }
    }
}
