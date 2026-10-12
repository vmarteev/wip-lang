#include <stdint.h>

typedef struct Thing Thing;

void listed(uint32_t *count, uint32_t *items);
void made(const int32_t *info, Thing **out);
int64_t summed(const uint32_t *items, int64_t count);
void cleared(void *at);

static inline void doubled(uint32_t *at) {
    *at *= 2;
}
