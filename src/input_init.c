#include "includes/input_init.h"
#include "includes/pop_module.h"

// Static callback for keyboard events
static keyboard_callback_t keyboard_callback = 0;

// IDT entry structure
struct IDT_entry {
    unsigned short int offset_lowerbits;
    unsigned short int selector;
    unsigned char zero;
    unsigned char type_attr;
    unsigned short int offset_higherbits;
};

// IDT table
struct IDT_entry IDT[256];

// Function to load IDT
extern void load_idt(unsigned long *idt_ptr);

// Simple scancode to ASCII mapping for basic keys
static const char scancode_to_ascii[] = {
    0, 0, '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '-', '=', 0,
    0, 'q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p', '[', ']', 0,
    0, 'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', ';', '\'', '`',
    0, '\\', 'z', 'x', 'c', 'v', 'b', 'n', 'm', ',', '.', '/', 0,
    '*', 0, ' '
};

// Read from I/O port
unsigned char inb(unsigned short port) {
    unsigned char value;
    __asm__ __volatile__("inb %1, %0" : "=a"(value) : "dN"(port));
    return value;
}

// Write to I/O port
void outb(unsigned short port, unsigned char data) {
    __asm__ __volatile__("outb %0, %1" : : "a"(data), "dN"(port));
}

// Keyboard interrupt handler
void keyboard_handler(void) {
    unsigned char status;
    unsigned char scancode;

    status = inb(KEYBOARD_STATUS_PORT);
    if (status & 0x01) {
        scancode = inb(KEYBOARD_DATA_PORT);
        if (keyboard_callback) {
            keyboard_callback(scancode);
        }
    }

    // Send End of Interrupt (EOI)
    outb(0x20, 0x20);
}

// IDT initialization
void idt_init(void) {
    unsigned long keyboard_address;
    unsigned long idt_address;
    unsigned long idt_ptr[2];

    // Get the address of keyboard_handler
    keyboard_address = (unsigned long)keyboard_handler;

    // Populate IDT entry for keyboard interrupt (IRQ1)
    IDT[0x21].offset_lowerbits = keyboard_address & 0xffff;
    IDT[0x21].selector = 0x08;  // Kernel code segment
    IDT[0x21].zero = 0;
    IDT[0x21].type_attr = 0x8e; // Interrupt gate
    IDT[0x21].offset_higherbits = (keyboard_address & 0xffff0000) >> 16;

    // Program PICs
    outb(0x20, 0x11);  // Initialize master PIC
    outb(0xA0, 0x11);  // Initialize slave PIC
    outb(0x21, 0x20);  // Master PIC vector offset
    outb(0xA1, 0x28);  // Slave PIC vector offset
    outb(0x21, 0x04);  // Tell master PIC about slave
    outb(0xA1, 0x02);  // Tell slave PIC its cascade identity
    outb(0x21, 0x01);  // Set master PIC to 8086 mode
    outb(0xA1, 0x01);  // Set slave PIC to 8086 mode

    // Mask interrupts except keyboard
    outb(0x21, 0xFD);  // Enable only IRQ1 (keyboard)
    outb(0xA1, 0xFF);  // Mask all interrupts on slave PIC

    // Load IDT
    idt_address = (unsigned long)IDT;
    idt_ptr[0] = (sizeof(struct IDT_entry) * 256) + ((idt_address & 0xffff) << 16);
    idt_ptr[1] = idt_address >> 16;
    load_idt(idt_ptr);
}

// Register keyboard callback
void register_keyboard_callback(keyboard_callback_t callback) {
    keyboard_callback = callback;
}

// Initialize keyboard
void init_keyboard(void) {
    idt_init();  // Initialize IDT with keyboard handler
}