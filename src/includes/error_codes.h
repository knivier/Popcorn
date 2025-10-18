// src/includes/error_codes.h
#ifndef ERROR_CODES_H
#define ERROR_CODES_H

// Error codes for consistent error handling across modules
typedef enum {
    ERR_SUCCESS = 0,
    ERR_NULL_POINTER = 1,
    ERR_INVALID_INPUT = 2,
    ERR_BUFFER_OVERFLOW = 3,
    ERR_NOT_FOUND = 4,
    ERR_ALREADY_EXISTS = 5,
    ERR_NO_SPACE = 6,
    ERR_PERMISSION_DENIED = 7,
    ERR_NAME_TOO_LONG = 8,
    ERR_INVALID_OPERATION = 9,
    ERR_UNKNOWN = 10
} ErrorCode;

// Error messages corresponding to error codes
static const char* error_messages[] = {
    "Success",
    "Null pointer error",
    "Invalid input",
    "Buffer overflow",
    "Not found",
    "Already exists",
    "No space available",
    "Permission denied",
    "Name too long",
    "Invalid operation",
    "Unknown error"
};

// Function to get error message from error code
static inline const char* get_error_message(ErrorCode code) {
    if (code >= 0 && code <= ERR_UNKNOWN) {
        return error_messages[code];
    }
    return "Invalid error code";
}

#endif // ERROR_CODES_H

