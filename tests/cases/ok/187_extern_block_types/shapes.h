#ifndef SHAPES_H
#define SHAPES_H
#include <stdint.h>

typedef struct Point { double x, y; } Point;
typedef struct Circle { Point at; double r; } Circle;

static inline double area(Circle c) { return 3.14159 * c.r * c.r; }
int64_t quadrant(Point p);
#endif
