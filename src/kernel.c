/*
*  kernel.c
*/
#include "includes/pop_module.h"
#include "includes/input_init.h"
#include "includes/spinner_pop.h"

extern const PopModule shimjapii_module;
extern const PopModule spinner_module;

// Define the scancode to ASCII mapping
const char scancode_to_ascii[128] = {
    0, 0, '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '-', '=', 0, 0,
    'q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p', '[', ']', '\n', 0,
    'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', ';', '\'', '`', 0, '\\',
    'z', 'x', 'c', 'v', 'b', 'n', 'm', ',', '.', '/', 0, '*', 0, ' '
};

// Keyboard event callback function (ISR)
void keyboard_event_isr(unsigned char scancode) {

    // Read the scancode from the keyboard controller
    scancode = inb(0x60);

    // Ignore key release events (scancode with the most significant bit set)
    if (scancode & 0x80) {
        return;
    }

    // For now, we'll just print the ASCII character if available
    if (scancode < 128 && scancode_to_ascii[scancode] != 0) {  // Standard scancode set is 128 keys
        char *vidptr = (char*)0xb8000;
        static unsigned int pos = 0;

        vidptr[pos] = scancode_to_ascii[scancode];
        vidptr[pos + 1] = 0x02;  // Green color
        pos += 2;
    }

    // Send End of Interrupt (EOI) signal to the PIC
    outb(0x20, 0x20);
}

// Define the __stack_chk_fail_local function to avoid linker errors
void __stack_chk_fail_local(void) {
    // This function is called when stack smashing is detected
    while (1) {
        __asm__ __volatile__("hlt");
    }
}

void kmain(void)
{
    const char *str = "Popcorn v0.3 Popped!!!";
    char *vidptr = (char*)0xb8000;     //video mem begins here.
    unsigned int i = 0;
    unsigned int j = 0;

    // Clear the screen
    while(j < 80 * 25 * 2) {
        vidptr[j] = ' ';
        vidptr[j+1] = 0x00;         
        j = j + 2;
    }

    j = 0;

    // Print the welcome message
    while(str[j] != '\0') {
        vidptr[i] = str[j];
        vidptr[i+1] = 0x02;  // Green color
        ++j;
        i = i + 2;
    }

    // Initialize keyboard
    init_keyboard();
    register_keyboard_callback(&keyboard_event_isr);

    // Enable interrupts
    __asm__ __volatile__("sti");

    // Register and execute the shimjapii module
    register_pop_module(&shimjapii_module);
    register_pop_module(&spinner_module);
    execute_all_pops(i);

    // Infinite loop to keep the kernel running
    while(1) {
        // Perform other tasks here
        // ...

        // Halt the CPU until the next interrupt
        __asm__ __volatile__("hlt");
    }
}