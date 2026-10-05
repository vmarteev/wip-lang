/* Structs by value, in and out — what Wip cannot call directly. */
struct Rect {
    int x;
    int y;
    int width;
    int height;
};

struct Rect grow(struct Rect r, int by) {
    struct Rect out = r;
    out.width += by;
    out.height += by;
    return out;
}

long long area(struct Rect r) {
    return (long long)r.width * (long long)r.height;
}
