/* A union as C writes it, so Wip's layout can be checked against C's own
   `sizeof`, and C can be handed one to fill. */
#include <stdint.h>

struct KeyboardEvent {
    uint32_t kind;
    uint32_t timestamp;
    int32_t keysym;
};

union Event {
    uint32_t kind;
    struct KeyboardEvent key;
    uint8_t padding[24];
};

int64_t event_size(void) { return (int64_t) sizeof(union Event); }
int64_t event_align(void) { return (int64_t) _Alignof(union Event); }

/* What a library does: fills in the event it was handed. */
void fill_key(union Event *event, int32_t keysym) {
    event->key.kind = 768;
    event->key.timestamp = 42;
    event->key.keysym = keysym;
}
