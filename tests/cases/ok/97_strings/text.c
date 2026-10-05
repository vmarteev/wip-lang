/* A `str` reaches C as two arguments: the bytes and how many there are.
 Nothing here expects a NUL. */
long long text_sum(const char *bytes, long long len) {
    long long total = 0;
    for (long long i = 0; i < len; i += 1) {
        total += (unsigned char) bytes[i];
    }
    return total;
}

long long elem_sum(const long long *xs, long long len) {
    long long total = 0;
    for (long long i = 0; i < len; i += 1) {
        total += xs[i];
    }
    return total;
}
