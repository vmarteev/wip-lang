#include "shapes.h"
size_t vertex_size(void) { return sizeof(vertex); }
size_t mixed_size(void) { return sizeof(mixed); }
size_t mixed_align(void) { return _Alignof(mixed); }
