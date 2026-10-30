/* Runs the non-deterministic instructions the random generator excludes (RDTSC,
 * RDTSCP, RDRAND with retry, RCPPS, RSQRTPS), checks the results are plausible and
 * prints only "ok". */
#include <stdint.h>
#include <stdio.h>

typedef float v4sf __attribute__((vector_size(16)));

static int near(float got, float want) {
    float err = (got - want) / want;
    return err < 0.001f && err > -0.001f;
}

int main(void) {
    uint32_t lo, hi, aux;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    uint64_t t0 = ((uint64_t)hi << 32) | lo;
    __asm__ volatile("rdtscp" : "=a"(lo), "=d"(hi), "=c"(aux));
    uint64_t t1 = ((uint64_t)hi << 32) | lo;
    if (t1 < t0) {
        fprintf(stderr, "rdtscp went back\n");
        return 1;
    }

    uint64_t r = 0;
    unsigned char ok = 0;
    for (int tries = 0; tries < 100 && !ok; tries++)
        __asm__ volatile("rdrand %0; setc %1" : "=r"(r), "=qm"(ok) : : "cc");
    if (!ok) {
        fprintf(stderr, "rdrand gave no value\n");
        return 1;
    }

    v4sf x = {1.0f, 2.0f, 4.0f, 0.25f};
    v4sf rcp, rsqrt;
    __asm__ volatile("rcpps %1, %0" : "=x"(rcp) : "x"(x));
    __asm__ volatile("rsqrtps %1, %0" : "=x"(rsqrt) : "x"(x));
    if (!near(rcp[1], 0.5f) || !near(rcp[3], 4.0f) || !near(rsqrt[2], 0.5f) ||
        !near(rsqrt[3], 2.0f)) {
        fprintf(stderr, "rcpps/rsqrtps out of range\n");
        return 1;
    }
    printf("ok\n");
    return 0;
}
