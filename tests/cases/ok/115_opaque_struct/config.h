#ifndef WIP_CONFIG_H
#define WIP_CONFIG_H
#include <stdint.h>

struct Config {
    unsigned mode : 3;
    unsigned level : 5;
    const char *name;
};

struct Config *config_new(void);
void config_free(struct Config *c);
#endif
