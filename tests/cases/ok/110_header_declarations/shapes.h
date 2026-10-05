/* What a library often gives instead of a symbol: a `static inline`
   function and a function-like macro. Neither can be linked against. */
#ifndef WIP_SHAPES_H
#define WIP_SHAPES_H

#include <stdint.h>

static inline int32_t doubled(int32_t value) {
    return value * 2;
}

#define LARGER(a, b) ((a) > (b) ? (a) : (b))

#endif
