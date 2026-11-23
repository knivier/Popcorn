// src/pop_module.h
#ifndef POP_MODULE_H
#define POP_MODULE_H

// Structure to define a pop module.
// Layout is mirrored by `PopModule` in src/rust/popcorn_kernel/src/abi.rs.
typedef struct {
    const char* name;
    const char* message;
    void (*pop_function)(unsigned int start_pos);
} PopModule;

// Record a pop in the registration catalog (called from Rust pops/registry.rs).
// The catalog is not iterated: pop_function is not run through it.
void register_pop_module(const PopModule* module);

#endif
