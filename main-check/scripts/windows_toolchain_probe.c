#include <math.h>
#include <stdlib.h>
#include <stdio.h>

_Static_assert(sizeof(void *) == 8, "Windows x64 toolchain required");

int main(void) {
    volatile char stack_probe[65536];
    stack_probe[0] = 0;
    stack_probe[sizeof(stack_probe) - 1] = 0;
    return stack_probe[0] + stack_probe[sizeof(stack_probe) - 1];
}
