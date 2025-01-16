// src/halt_pop.c

// Function to introduce a delay
static void delay(unsigned int milliseconds) {
    for (volatile unsigned int i = 0; i < milliseconds * 1000; i++);
}

// Function to display the halt message with animation
void halt_pop_func(unsigned int start_pos) {
    char* vidptr = (char*)0xb8000;
    const char* msg = "System Halted, Press Enter to Continue";
    unsigned int i = start_pos;
    unsigned int j = 0;

    // Clear the screen with green background
    for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
        vidptr[m] = ' ';
        vidptr[m + 1] = 0x20; // Green background with black text
    }

    // Display the halt message
    while (msg[j] != '\0') {
        vidptr[i] = msg[j];
        vidptr[i + 1] = 0x2C; // Green background with red text
        ++j;
        i += 2;
    }

    // Animation effect (simple blinking)
    for (int blink = 0; blink < 5; ++blink) {
        for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
            vidptr[m + 1] ^= 0x08; // Toggle blink bit
        }
        delay(500);
    }

        // Rainbow color animation
        for (int rainbow = 0; rainbow < 8; rainbow++) {
            for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
                vidptr[m + 1] = (rainbow << 4) | 0x0F;
            }
            delay(300);
        }

        // Fade to black
        for (int fade = 0x0F; fade >= 0; fade--) {
            for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
                vidptr[m + 1] = fade;
            }
            delay(100);
        }
}

// Define the PopModule structure for halt_pop
const struct {
    const char* name;
    const char* message;
    void (*pop_function)(unsigned int start_pos);
} halt_module = {
    .name = "halt",
    .message = "Displays a Green Screen of Death",
    .pop_function = halt_pop_func
};