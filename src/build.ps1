# Set object directory
$OBJ_DIR = "obj"

# Function to clean previous build
function Clean-Build {
    Remove-Item -Recurse -Force $OBJ_DIR -ErrorAction SilentlyContinue
    Remove-Item -Force kernel -ErrorAction SilentlyContinue
}

# Function to compile and link the kernel
function Build-Kernel {
    New-Item -ItemType Directory -Force -Path $OBJ_DIR | Out-Null

    # Compile assembly files
    nasm -f elf32 kernel.asm -o "$OBJ_DIR/kasm.o"
    if ($LASTEXITCODE -ne 0) { exit 1 }

    nasm -f elf32 idt.asm -o "$OBJ_DIR/idt.o"
    if ($LASTEXITCODE -ne 0) { exit 1 }

    # Compile C files
    $cFiles = @("kernel.c", "console.c", "pop_module.c", "shimjapii_pop.c", "spinner_pop.c", "uptime_pop.c", "halt_pop.c", "filesystem_pop.c")

    foreach ($file in $cFiles) {
        $outputFile = "$OBJ_DIR/" + [System.IO.Path]::GetFileNameWithoutExtension($file) + ".o"
        gcc -m32 -c $file -o $outputFile -Wall -Wextra -fno-stack-protector -fno-pie -nostdlib
        if ($LASTEXITCODE -ne 0) { exit 1 }
    }

    # Link files
    ld -m i386pe -T link.ld -o kernel `
        "$OBJ_DIR/kasm.o" `
        "$OBJ_DIR/kernel.o" `
        "$OBJ_DIR/console.o" `
        "$OBJ_DIR/pop_module.o" `
        "$OBJ_DIR/shimjapii_pop.o" `
        "$OBJ_DIR/idt.o" `
        "$OBJ_DIR/spinner_pop.o" `
        "$OBJ_DIR/uptime_pop.o" `
        "$OBJ_DIR/halt_pop.o" `
        "$OBJ_DIR/filesystem_pop.o"
}

# Function to run the kernel in QEMU
function Run-Kernel {
    qemu-system-i386 -kernel kernel
}

# Execute build process
Clean-Build
Build-Kernel
Run-Kernel
