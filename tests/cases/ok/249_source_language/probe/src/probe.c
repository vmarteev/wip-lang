/* Answers 1 where it is compiled as Objective-C, and 0 as C. */
int probe_objc(void) {
#ifdef __OBJC__
    return 1;
#else
    return 0;
#endif
}
