/* C strings the program reads back: the bytes belong to C, and `toStr()`
   borrows them. */
#include <stdlib.h>
#include <string.h>

const char *text_greeting(void) {
    return "hello from C";
}

const char *text_empty(void) {
    return "";
}

char *text_repeat(const char *s, long long times) {
    size_t len = strlen(s);
    char *out = malloc(len * (size_t) times + 1);
    out[0] = '\0';
    for (long long i = 0; i < times; i += 1) {
        memcpy(out + len * (size_t) i, s, len);
    }
    out[len * (size_t) times] = '\0';
    return out;
}

void text_free(char *s) {
    free(s);
}
