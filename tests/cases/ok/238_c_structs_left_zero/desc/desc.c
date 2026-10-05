#include <stdio.h>
#include "desc.h"
int run(const Desc *d) {
    int got = d->width * 1000 + d->height;
    if (d->frame_cb) { d->frame_cb(); got += 100000; }
    if (d->event_cb) { got += 10000000 * d->event_cb(2); }
    /* `%p` of a null is `0x0` on a Mac and `(nil)` with glibc, so the
       pointer is said as whether it is one. */
    printf("title %s inner %d %.1f counts %d %d %d user %s\n", d->title ? d->title : "(null)", d->inner.a, d->inner.b, d->counts[0], d->counts[1], d->counts[2], d->user ? "set" : "null");
    return got;
}
