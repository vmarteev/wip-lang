/* Bytes of 2 where Wip expects a `bool`: in a struct's fields, as C
   leaves them when it fills a struct from a file or forgets a field; in a
   variable; and from functions that declare the flag as a byte. */
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

typedef uint8_t (*answer_fn)(void);

typedef struct {
    int32_t n;
    bool flag;
    bool flags[2];
    answer_fn answer;
} Rec;

static void two(void *p) {
    uint8_t t = 2;
    memcpy(p, &t, 1);
}

static uint8_t answer_two(void) {
    return 2;
}

void fill(Rec *r) {
    r->n = 1;
    two(&r->flag);
    two(&r->flags[0]);
    two(&r->flags[1]);
    r->answer = answer_two;
}

Rec filled(void) {
    Rec r;
    fill(&r);
    return r;
}

bool shared_flag;

void set_shared_flag(void) {
    two(&shared_flag);
}

/* The callee's flag declared as a byte, as a binding of a `uint8_t` might. */
typedef int32_t (*take_fn)(uint8_t);

int32_t call_with_two(take_fn f) {
    return f(2);
}

int32_t takes_flag(uint8_t b);

int32_t call_export_with_two(void) {
    return takes_flag(2);
}
