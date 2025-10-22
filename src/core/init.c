#include "../includes/init.h"
#include "../includes/console.h"
#include "../includes/timer.h"
#include "../includes/scheduler.h"
#include "../includes/memory.h"
#include "../includes/pop_module.h"
#include "../includes/multiboot2.h"
#include "../includes/sysinfo_pop.h"
#include "../includes/memory_pop.h"
#include "../includes/cpu_pop.h"
#include "../includes/dolphin_pop.h"
#include "../includes/spinner_pop.h"
#include "../includes/syscall.h"
#include <stddef.h>

extern void int_to_str(int num, char *str);
extern void multiboot2_parse(void);
extern void idt_init(void);
extern void kb_init(void);
extern void register_pop_module(const PopModule* module);
extern void timer_init(uint32_t frequency_hz);
extern void timer_set_tick_handler(void (*handler)(void));
extern void timer_enable(void);
extern void scheduler_init(void);
extern void scheduler_tick(void);
extern void memory_init(void);
extern void syscall_init(void);
extern void console_draw_header(const char* title);
extern void console_draw_prompt_with_path(const char* path);
extern void console_print_status_bar(void);
extern const char* get_current_directory(void);

extern uint64_t multiboot2_info_ptr;
extern ConsoleState console_state;

static const int total_init_steps = 9;

void init_boot_screen(void) {

    console_init();

    init_draw_header();

    init_show_memory_info();

    init_show_timer_info();

    init_show_scheduler_info();

    init_show_syscall_info();

    init_show_modules();

    init_draw_progress_bar(total_init_steps, total_init_steps, "Initialization Complete");
    console_println_color("", CONSOLE_FG_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);

    init_wait_for_enter();

    init_transition_to_console();
}

void init_draw_header(void) {
    console_clear();

    console_set_cursor(0, 0);
    for (int i = 0; i < BOOT_SCREEN_WIDTH; i++) {
        console_print_color("=", BOOT_TITLE_COLOR);
    }

    console_set_cursor(0, 1);
    console_print_color("=", BOOT_TITLE_COLOR);
    console_set_cursor(BOOT_SCREEN_WIDTH - 1, 1);
    console_print_color("=", BOOT_TITLE_COLOR);

    console_set_cursor(0, 2);
    console_print_color("=", BOOT_TITLE_COLOR);

    console_set_cursor(15, 2);
    console_println_color("POPCORN KERNEL v0.5", BOOT_TITLE_COLOR);

    console_set_cursor(0, 3);
    console_print_color("=", BOOT_TITLE_COLOR);
    console_set_cursor(BOOT_SCREEN_WIDTH - 1, 3);
    console_print_color("=", BOOT_TITLE_COLOR);

    console_set_cursor(0, 4);
    for (int i = 0; i < BOOT_SCREEN_WIDTH; i++) {
        console_print_color("=", BOOT_TITLE_COLOR);
    }

    console_set_cursor(0, 6);
    console_println_color("Modular Kernel Framework", BOOT_SUBTITLE_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);

    console_set_cursor(0, 8);
    console_println_color("Architecture: x86-64 (Long Mode)", BOOT_INFO_COLOR);
    console_println_color("Memory Management: Virtual Memory + Heap Allocator", BOOT_INFO_COLOR);
    console_println_color("Scheduling: Preemptive Multi-Task Scheduler", BOOT_INFO_COLOR);
    console_println_color("Interrupts: Timer-Driven (100Hz) + Hardware IRQs", BOOT_INFO_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);

    console_set_cursor(0, 14);
    console_println_color("Initialization Progress:", BOOT_SUBTITLE_COLOR);
    console_draw_separator(15, BOOT_INFO_COLOR);
}

void init_draw_progress_bar(int current, int total, const char* item) {
    char buffer[64];
    int progress_percent = (current * 100) / total;

    console_set_cursor(0, 16);
    console_print_color("[", BOOT_INFO_COLOR);

    int bar_width = 50;
    int filled = (current * bar_width) / total;

    for (int i = 0; i < bar_width; i++) {
        if (i < filled) {
            console_print_color("=", BOOT_SUCCESS_COLOR);
        } else {
            console_print_color("-", BOOT_INFO_COLOR);
        }
    }

    console_print_color("] ", BOOT_INFO_COLOR);

    int_to_str(progress_percent, buffer);
    console_print_color(buffer, BOOT_SUCCESS_COLOR);
    console_print_color("% ", BOOT_INFO_COLOR);

    console_print_color(item, BOOT_INFO_COLOR);

    for (int i = 0; i < 20; i++) {
        console_print_color(" ", CONSOLE_FG_COLOR);
    }
}

void init_show_memory_info(void) {
    init_draw_progress_bar(1, total_init_steps, "Initializing Memory Management");

    memory_init();

    console_set_cursor(0, 18);
    console_print_color("  ✓ Memory Management System", BOOT_SUCCESS_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);

    KernelMemoryStats* stats = memory_get_stats();
    char buffer[32];

    console_set_cursor(0, 19);
    console_print_color("    Total Memory: ", BOOT_INFO_COLOR);
    int_to_str((int)(stats->total_bytes / (1024 * 1024)), buffer);
    console_print_color(buffer, BOOT_SUCCESS_COLOR);
    console_println_color(" MB", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 20);
    console_print_color("    Available: ", BOOT_INFO_COLOR);
    int_to_str((int)(stats->free_bytes / (1024 * 1024)), buffer);
    console_print_color(buffer, BOOT_SUCCESS_COLOR);
    console_println_color(" MB", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 21);
    for (int i = 0; i < BOOT_SCREEN_WIDTH; i++) {
        console_print_color(" ", CONSOLE_FG_COLOR);
    }
}

void init_show_timer_info(void) {
    init_draw_progress_bar(2, total_init_steps, "Initializing Timer System");

    timer_init(TIMER_FREQUENCY);

    console_set_cursor(0, 18);
    console_print_color("  ✓ Programmable Interval Timer (PIT)", BOOT_SUCCESS_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);

    console_set_cursor(0, 19);
    console_print_color("    Frequency: ", BOOT_INFO_COLOR);
    char buffer[16];
    int_to_str(TIMER_FREQUENCY, buffer);
    console_print_color(buffer, BOOT_SUCCESS_COLOR);
    console_println_color(" Hz", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 20);
    console_print_color("    Resolution: ", BOOT_INFO_COLOR);
    int_to_str(1000 / TIMER_FREQUENCY, buffer);
    console_print_color(buffer, BOOT_SUCCESS_COLOR);
    console_println_color(" ms", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 21);
    for (int i = 0; i < BOOT_SCREEN_WIDTH; i++) {
        console_print_color(" ", CONSOLE_FG_COLOR);
    }
}

void init_show_scheduler_info(void) {
    init_draw_progress_bar(3, total_init_steps, "Initializing Scheduler");

    scheduler_init();

    console_set_cursor(0, 18);
    console_print_color("  ✓ Preemptive Multi-Task Scheduler", BOOT_SUCCESS_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);

    console_set_cursor(0, 19);
    console_print_color("    Priority Levels: ", BOOT_INFO_COLOR);
    console_println_color("5 (Idle → Realtime)", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 20);
    console_print_color("    Scheduling: ", BOOT_INFO_COLOR);
    console_println_color("Round-Robin with Priority", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 21);
    for (int i = 0; i < BOOT_SCREEN_WIDTH; i++) {
        console_print_color(" ", CONSOLE_FG_COLOR);
    }
}

void init_show_syscall_info(void) {
    init_draw_progress_bar(4, total_init_steps, "Initializing System Call Interface");

    syscall_init();

    console_set_cursor(0, 18);
    console_print_color("  ✓ System Call Interface", BOOT_SUCCESS_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);

    console_set_cursor(0, 19);
    console_print_color("    Interrupt: ", BOOT_INFO_COLOR);
    console_println_color("0x80 (User Accessible)", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 20);
    console_print_color("    Calls: ", BOOT_INFO_COLOR);
    console_println_color("21 System Calls Registered", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 21);
    for (int i = 0; i < BOOT_SCREEN_WIDTH; i++) {
        console_print_color(" ", CONSOLE_FG_COLOR);
    }
}

void init_show_modules(void) {
    init_draw_progress_bar(5, total_init_steps, "Loading Kernel Modules");

    extern const PopModule spinner_module;
    extern const PopModule uptime_module;
    extern const PopModule halt_module;
    extern const PopModule filesystem_module;
    extern const PopModule sysinfo_module;
    extern const PopModule memory_module;
    extern const PopModule cpu_module;
    extern const PopModule dolphin_module;
    extern const PopModule shimjapii_module;

    register_pop_module(&spinner_module);
    register_pop_module(&uptime_module);
    register_pop_module(&filesystem_module);
    register_pop_module(&sysinfo_module);
    register_pop_module(&memory_module);
    register_pop_module(&cpu_module);
    register_pop_module(&dolphin_module);
    register_pop_module(&halt_module);
    register_pop_module(&shimjapii_module);

    console_set_cursor(0, 18);
    console_print_color("  ✓ Kernel Modules Loaded", BOOT_SUCCESS_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);

    console_set_cursor(0, 19);
    console_print_color("    Modules: ", BOOT_INFO_COLOR);
    console_println_color("9 Pop Modules Registered", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 20);
    console_print_color("    Features: ", BOOT_INFO_COLOR);
    console_println_color("Console, Filesystem, Editor, System Info", BOOT_SUCCESS_COLOR);

    console_set_cursor(0, 21);
    for (int i = 0; i < BOOT_SCREEN_WIDTH; i++) {
        console_print_color(" ", CONSOLE_FG_COLOR);
    }
}

void init_wait_for_enter(void) {
    console_set_cursor(0, 22);
    console_println_color("", CONSOLE_FG_COLOR);
    console_println_color("Press ENTER to continue to console...", BOOT_SUBTITLE_COLOR);

    extern char read_port(unsigned short port);

    #define KEYBOARD_STATUS_PORT 0x64
    #define KEYBOARD_DATA_PORT 0x60
    #define ENTER_KEY_CODE 0x1C

    while (1) {
        unsigned char status = read_port(KEYBOARD_STATUS_PORT);
        if (status & 0x01) {
            char keycode = read_port(KEYBOARD_DATA_PORT);
            if (keycode == ENTER_KEY_CODE) {
                break;
            }
        }
    }
}

void init_transition_to_console(void) {
    console_clear();
    console_draw_header("Popcorn Kernel v0.5");
    console_println_color("Welcome to Popcorn Kernel!", CONSOLE_SUCCESS_COLOR);
    console_newline();
    multiboot2_parse();

    if (multiboot2_info_ptr == 0) {
        console_println_color("Warning: No Multiboot2 info received", CONSOLE_WARNING_COLOR);
    }

    idt_init();
    kb_init();

    timer_set_tick_handler(scheduler_tick);

    timer_enable();

    console_draw_prompt_with_path(get_current_directory());

    console_print_status_bar();
    }