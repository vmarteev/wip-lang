#include <stddef.h>

struct Rect {
    int x;
    int y;
    int width;
    int height;
};

long long area(const struct Rect *r) {
    return (long long)r->width * (long long)r->height;
}

void grow(struct Rect *r, int by) {
    r->width += by;
    r->height += by;
}

long long size_of_rect(void) { return (long long)sizeof(struct Rect); }
long long offset_of_height(void) { return (long long)offsetof(struct Rect, height); }
