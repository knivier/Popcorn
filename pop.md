The core idea is that each "pop" is a module that records itself in the kernel's pop catalog. Here's how it works in Popcorn v0.7:

**Rust pops (v0.7):** All pops (Shimjapii, Spinner, Uptime, Memory, CPU, Sysinfo, Dolphin) live in `src/rust/popcorn_kernel/src/pops/`. They export the same `PopModule` C ABI and register via `rust_pops_register()`. Prefer `console_ffi` helpers (`print`, `println_color`, `error`, `CursorGuard`) over raw VGA writes.

**Direction:** pops are UX/aggregation; durable I/O is drivers. The filesystem is no longer a pop: it is Rust FAT32 (`src/rust/popcorn_kernel/src/fs/`) on the selected block disk. Memory/CPU/clock are info drives (`mem`, `cpu`, `clock`); Sysinfo aggregates those drives. Halt/`hang` toys are gone.

1. The PopModule structure (in includes/pop_module.h) defines what a pop is:
```c
typedef struct {
    const char* name;        // Name of the pop module
    const char* message;     // Message it will display
    void (*pop_function)(unsigned int start_pos);  // Function that does the actual work
} PopModule;
```

2. We maintain a simple registry system in pop_module.c:
```c
static PopModule* modules[MAX_MODULES];  // Array to store all registered pops
static int module_count = 0;            // Keep track of how many we have
```

3. Console System Integration (v0.5+):
   - All pops should use the console system (includes/console.h) instead of writing directly to VGA memory
   - This ensures consistent behavior and prevents interference with user input
   - Each pop must save and restore the console state (cursor position, color)

4. To add a new pop, you:
   - Preferred: create a new Rust module in src/rust/popcorn_kernel/src/pops/ (like uptime.rs) that defines a `#[no_mangle]` PopModule static (C ABI from crate::abi), and add `mod <name>;` to pops/mod.rs
   - Use console_ffi helpers (print, println_color, CursorGuard) for output and cursor save/restore
   - Register it by adding a register_pop_module(...) line to pop_register_all() in src/rust/popcorn_kernel/src/pops/registry.rs (called at boot via init_show_modules() -> rust_pops_register() in src/core/init.c), not in kmain()
   - Rust pops need no build-script change; they build with the Rust crate
   - Legacy C pop (none remain, but the same PopModule ABI still works): include console.h and utils.h, write the function with cursor save/restore, define a PopModule, and add a `compile_c` line for the .c file to `scripts/lib/kernel.sh` (the Python builders in `scripts/popcorn_build/` only wrap `scripts/core.sh` and need no change)

For example, if you wanted to add a new "beep" pop as a C pop, you could create beep_pop.c:
```c
#include "includes/pop_module.h"
#include "includes/console.h"
#include "includes/utils.h"

// Access console state to save/restore cursor
extern ConsoleState console_state;

void beep_pop_func(unsigned int start_pos) {
    (void)start_pos;  // Unused parameter
    
    // Save current console state
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;
    
    // Display message at specific location
    console_set_cursor(10, 5);
    console_print_color("BEEP! BEEP!", CONSOLE_SUCCESS_COLOR);
    
    // Optional: Add a delay for visibility
    util_delay(100);
    
    // Restore previous cursor/color so normal typing is unaffected
    console_set_color(prev_color);
    console_set_cursor(prev_x, prev_y);
}

const PopModule beep_module = {
    .name = "beep",
    .message = "BEEP! BEEP!",
    .pop_function = beep_pop_func
};
```

Then record it in the catalog from init_show_modules() in src/core/init.c, which already declares register_pop_module as extern. For a Rust pop, add a line to pop_register_all() in src/rust/popcorn_kernel/src/pops/registry.rs instead. Registration alone never runs the pop, so also call beep_pop_func from the shell command or drive that should show it:
```c
extern const PopModule beep_module;
register_pop_module(&beep_module);   /* inside init_show_modules() */
```

When the kernel runs:
1. It boots up and initializes the console system
2. During initialization, init_show_modules() calls rust_pops_register(), which registers every pop centrally via register_pop_module() (see src/rust/popcorn_kernel/src/pops/registry.rs)
3. Pops are not run through the catalog: registration only records them, and each pop's behavior comes from its own exported helpers, called from shell commands, drives and the Dolphin editor (the registered pop_function entry point is never called)
4. Each pop saves the cursor state, displays its content, then restores the cursor

The system is designed so:
- Adding a pop means declaring it in pops/mod.rs, registering it in pops/registry.rs, and calling it from a shell command or drive (the registry does not run pops); other pops never need to change
- Each pop can have its own styling (colors, messages, positions)
- Registration only records a pop in the catalog (capped at 10 entries); the catalog is never iterated, so pops run from their own call sites, not in registration order
- The console system handles all screen management automatically
- Cursor save/restore prevents pops from interfering with user input
- Shared utilities (like util_delay) reduce code duplication

Best Practices for Pop Development:
- Always include console.h for display operations
- Always save and restore console state (cursor position and color)
- Mark unused parameters with (void)parameter to avoid warnings
- Use console color constants (CONSOLE_SUCCESS_COLOR, etc.) for consistency
- Use util_delay() from utils.h instead of creating your own delay function
- Test that your pop doesn't interfere with keyboard input