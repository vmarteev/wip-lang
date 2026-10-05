/* Structs whose padding C decides, and what C's sizeof and _Alignof say
   of them. */
#include <stddef.h>
#include <stdint.h>
typedef struct vertex {
    float x, y, u, v;
    uint32_t attr;
} vertex;
typedef struct mixed {
    uint8_t a;
    double b;
    uint16_t c;
} mixed;
size_t vertex_size(void);
size_t mixed_size(void);
size_t mixed_align(void);
