// shimjapii.c
void shimjapii_popped(unsigned int start_pos) {
    const char *msg = "Shimjapii popped";  // Message to be printed
    char *vidptr = (char*)0xb8000;  // Video memory
    unsigned int i = start_pos;  // Start from the position passed by the calling function
    
    // Move to the next line
    i = (i / (80 * 2) + 1) * (80 * 2);  // Calculate the start of the next line

    // Write the message to the video memory
    unsigned int j = 0;
    while (msg[j] != '\0') {
        vidptr[i] = msg[j];       // Write character to video memory
        vidptr[i + 1] = 0x07;     // Light grey on black background
        ++j;
        i = i + 2;                // Move to the next position in video memory
    }
}
