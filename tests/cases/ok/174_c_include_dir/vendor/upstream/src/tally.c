/* The library as released. Compiled on its own it would scale by 1; it is
   not a module, so it is compiled only through vendor/tally.c. */
#include "tally.h"

#ifndef TALLY_SCALE
#define TALLY_SCALE 1
#endif

int tally_add(int a, int b) { return a + b; }

int tally_scale(void) { return TALLY_SCALE; }
