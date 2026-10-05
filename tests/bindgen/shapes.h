/* A header with one of everything the generator writes. */
#ifndef WIP_SHAPES_H
#define WIP_SHAPES_H

#include <stdint.h>

#define SHAPES_VERSION 3
#define SHAPES_NAME "shapes"
#define SHAPES_MASK 0x0f
#ifdef SHAPES_WIDE
#define SHAPES_WIDTH 64
#else
#define SHAPES_WIDTH 32
#endif
#define SHAPES_AREA(w, h) ((w) * (h))

/* Constants that are not literals, which clang works out:
   a float, arithmetic on one, on an enumerator, a second name for one,
   two structs, and a function's second name, which is none. */
#define SHAPES_SCALE 1.5f
#define SHAPES_HALF (SHAPES_SCALE / 2.0f)
#define SHAPES_LAST (BOTTOM_RIGHT + 1)
#define SHAPES_FIRST TOP_LEFT
#define SHAPES_ORIGIN ((struct Point){ 0, 0 })
#define SHAPES_CORNER ((struct Point){ -1, 2 })
#define shapes_make canvas_new

struct Point {
    int x;
    int y;
};

union Value {
    int32_t number;
    double fraction;
    uint8_t bytes[8];
};

/* Declared, never defined: a handle a library hands back. */
struct Canvas;

struct Style {
    unsigned weight : 3;
    unsigned italic : 1;
    const char *family;
};

enum Corner { TOP_LEFT, TOP_RIGHT, BOTTOM_LEFT = 10, BOTTOM_RIGHT };

extern int shapes_errors;

struct Canvas *canvas_new(int width, int height);
void canvas_free(struct Canvas *canvas);
int canvas_draw(struct Canvas *canvas, const struct Point *at, double scale);
int canvas_printf(struct Canvas *canvas, const char *format, ...);

static inline int point_sum(struct Point p) { return p.x + p.y; }

int canvas_sort(struct Canvas *canvas, int (*compare)(const void *, const void *));

// A second name for a type the header already declares:
// a use of it is read as the first, however many names deep.
typedef struct Point Position;
typedef Position Spot;
typedef struct Point *PointRef;

// A pointer to pointers: only the outermost may be a reference, since
// nothing may hold one.
void point_collect(struct Point **found, int count);
struct Point spot_move(Spot from, int dx, int dy);
void point_shift(PointRef p, int dx, int dy);

#endif
