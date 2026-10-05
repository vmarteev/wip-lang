typedef struct Inner { int a; float b; } Inner;
typedef struct Desc {
    int width;
    int height;
    const char *title;
    void (*frame_cb)(void);
    int (*event_cb)(int);
    Inner inner;
    int counts[3];
    void *user;
} Desc;
int run(const Desc *d);
