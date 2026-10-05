/* Calls `step` once for each of `count` numbers, with the `void *` it was
   given, as a C library calls a program back. */
typedef void (*each_step)(void *data, int n);
typedef struct each_desc {
    int count;
    each_step step;
    void *user_data;
} each_desc;
void each_run(const each_desc *desc);
