/* Prints its arguments, one environment variable and a double computation, then exits
 * with 3. Compared byte for byte against native (tests/static_hello.rs, dynamic.py). */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    printf("argc=%d\n", argc);
    for (int i = 0; i < argc; i++)
        printf("argv[%d]=%s\n", i, argv[i]);
    const char *name = getenv("CALCO_HELLO");
    printf("CALCO_HELLO=%s\n", name ? name : "(unset)");

    /* Partial sum of 1/k^2 and its product with argc, so nothing folds at compile time. */
    double sum = 0.0;
    for (int k = 1; k <= 1000 + argc; k++)
        sum += 1.0 / ((double)k * (double)k);
    printf("sum=%.15f scaled=%.6e\n", sum, sum * argc / 3.0);
    return 3;
}
