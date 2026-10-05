#include <stdlib.h>
#include <stdio.h>
#include "shapes.h"

int shapes_errors = 0;

static struct Style the_style;

struct Style *style_new(void) {
    the_style.weight = 2;
    the_style.italic = 1;
    the_style.family = "serif";
    return &the_style;
}

struct Canvas { int width; int height; };

struct Canvas *canvas_new(int width, int height) {
    struct Canvas *c = malloc(sizeof *c);
    c->width = width; c->height = height;
    return c;
}
void canvas_free(struct Canvas *c) { free(c); }
int canvas_draw(struct Canvas *c, const struct Point *at, double scale) {
    return (int) ((at->x + at->y + c->width) * scale);
}
int canvas_printf(struct Canvas *c, const char *format, ...) { (void) c; (void) format; return 7; }
int canvas_sort(struct Canvas *c, int (*cmp)(const void *, const void *)) {
    (void) c;
    int a = 1, b = 2;
    return cmp(&a, &b) * 10 + cmp(&b, &a);
}
