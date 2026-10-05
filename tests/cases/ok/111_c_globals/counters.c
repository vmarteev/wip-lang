/* Variables C owns, which Wip reads and writes through the accessors the
   compiler writes for it. */
int counter = 7;
const char *greeting = "hello from C";

int doubled_counter(void) {
    return counter * 2;
}
