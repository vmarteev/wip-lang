#include "shapes.h"

int64_t quadrant(Point p) {
    if (p.x >= 0 && p.y >= 0) return 1;
    if (p.x < 0 && p.y >= 0) return 2;
    if (p.x < 0) return 3;
    return 4;
}
