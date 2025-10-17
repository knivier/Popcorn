The core idea is that each "pop" is a module that can register itself with the kernel. Here's how it works in Popcorn v0.5:

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
   - Create a new .c file (like beep_pop.c)
   - Include console.h and utils.h
   - Write your pop function with proper cursor save/restore
   - Define a PopModule structure
   - Register it in kmain() using register_pop_module()
   - Add the .c file to all build scripts (build.sh, trymake.sh, build.ps1)

For example, if you wanted to add a new "beep" pop, you could create beep_pop.c:
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

Then in kernel.c, just add:
```c
extern const PopModule beep_module;
register_pop_module(&beep_module);
```

When the kernel runs:
1. It boots up and initializes the console system
2. During initialization, modules register themselves
3. Pops can be called at any time to display information
4. Each pop saves the cursor state, displays its content, then restores the cursor

The system is designed so:
- New pops can be added without changing existing code
- Each pop can have its own styling (colors, messages, positions)
- Pops are executed in registration order
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