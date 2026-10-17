/* A header with one of everything the generator writes. */
#ifndef WIP_SHAPES_H
#define WIP_SHAPES_H

#include <stdint.h>
#include "elsewhere.h"

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
/* More than an i64 holds, as a size meaning "the rest" is written. */
#define SHAPES_ALL (~0ULL)

struct Point {
    int x;
    int y;
};

union Value {
    int32_t number;
    double fraction;
    uint8_t bytes[8];
};

/* Fields named as Wip's keywords, which keep their names: a keyword
   names a field. */
struct Event {
    int type;
    float when;
};

/* Declared, never defined: a handle a library hands back. */
struct Canvas;

struct Style {
    unsigned weight : 3;
    unsigned italic : 1;
    const char *family;
};

/* Laid out by attributes: packed, a field aligned, a struct aligned, and a
   packing that is not 1, which is said as a layout left to C. */
struct __attribute__((packed)) Chunk {
    uint8_t kind;
    uint32_t length;
};

struct Light {
    float intensity;
    _Alignas(16) float direction[3];
};

/* An array of arrays. */
struct Transform {
    float matrix[3][4];
};

struct __attribute__((aligned(64))) Counters {
    long done;
};

#pragma pack(push, 2)
struct Pair {
    char tag;
    int value;
};
#pragma pack(pop)

enum Corner { TOP_LEFT, TOP_RIGHT, BOTTOM_LEFT = 10, BOTTOM_RIGHT };

// An enumerator past an `int` makes the enum's `unsigned int`; a `static
// const` number is a constant, as a 64-bit flag past any enum is written.
enum Layer { LAYER_BASE = 1, LAYER_TOP = 0x80000000u };
static const uint64_t SHAPES_STAGE_TOP = 0x8000000000000000ULL;
static const float SHAPES_RATIO = 1.5f;

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

// A list of names, as a library is given the extensions it is to load:
// a pointer that is itself `const` is the pointer.
int canvas_load(struct Canvas *canvas, const char *const *names, uint32_t count);

// What a header says of a pointer's null: one that may be null is a
// pointer, as is one the command line names; one that may not is read as
// if nothing were said. A struct C writes is a pointer too, since it is as
// often an array, or null.
int point_near(const struct Point *_Nullable to, const struct Point *_Nonnull from);
void point_label(struct Point *_Nonnull out, const char *_Nullable label);
void canvas_hint(struct Canvas *canvas, const struct Point *hint);

// An array of structs after its count, which may be empty and null, is a
// pointer; a struct after another number is one struct.
void canvas_plot(struct Canvas *canvas, uint32_t pointCount, const struct Point *points);
int point_steps(int steps, const struct Point *from);

// Structs another header declares: one pointed at is a type whose contents
// the binding does not know, and one held by value leaves its holder out.
struct Brush {
    const Palette *palette;
};
struct Stamp {
    Palette palette;
};
/* Holding a struct of bitfields, which C alone lays out, by value; and
   holding that. */
struct Styled {
    struct Style style;
};
struct Restyled {
    struct Styled styled;
};

#endif
