#ifndef WIP_OLDER_H
#define WIP_OLDER_H

struct Kept {
    unsigned flags : 4;
    unsigned count : 12;
};

struct Kept *kept_new(void);
void kept_free(struct Kept *kept);
#endif
