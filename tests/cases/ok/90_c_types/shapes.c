/* C's own types, seen from C: the binding beside this file must name them
   `c_int`, `c_long` and `size_t` rather than guess. */
#include <stddef.h>

int area(int width, int height) {
    return width * height;
}

long widen(int x) {
    return (long) x * 1000000000L;
}

size_t count_bytes(const char *text) {
    size_t n = 0;
    while (text[n] != '\0') {
        n += 1;
    }
    return n;
}

unsigned char byte_at(const char *text, size_t i) {
    return (unsigned char) text[i];
}
