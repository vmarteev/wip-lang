/* Functions that take a pointer that may be null, and call a Wip function
   that takes one, with a pointer and with null. */
#include <stddef.h>
#include <stdint.h>

typedef struct {
    int32_t width;
    int32_t height;
} Size;

int32_t area_or(const Size *size, int32_t otherwise) {
    return size == NULL ? otherwise : size->width * size->height;
}

void grow(Size *size) {
    if (size != NULL) {
        size->width += 1;
        size->height += 1;
    }
}

int32_t wip_area(const Size *size);

int32_t call_wip_area(void) {
    Size size = {3, 5};
    return wip_area(&size) * 100 + wip_area(NULL);
}
