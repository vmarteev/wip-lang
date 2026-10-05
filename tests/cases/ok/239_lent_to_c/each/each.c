#include "each.h"
void each_run(const each_desc *desc) {
    for (int i = 1; i <= desc->count; i++) {
        desc->step(desc->user_data, i);
    }
}
