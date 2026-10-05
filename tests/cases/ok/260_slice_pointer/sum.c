/* What a C library that takes a pointer and a count does with them.
 */
int sum_bytes(const unsigned char *bytes, int count) {
    int sum = 0;
    for (int i = 0; i < count; i++) {
        sum += bytes[i];
    }
    return sum;
}
