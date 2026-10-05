/* A single-header library: the declarations for everyone, and the
   implementation where TINY_IMPL is defined, once in a program. */
#ifndef TINY_H
#define TINY_H
int tiny_answer(int base);
#endif

#ifdef TINY_IMPL
int tiny_answer(int base) {
    return base + TINY_STEP;
}
#endif
