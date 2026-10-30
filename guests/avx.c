/* Prints the AVX-related CPUID bits and then executes vaddps. Under calco CPUID
 * announces none of them and vaddps (VEX) is out of scope: the guest dies by SIGILL
 * right after the line of guests/avx.expected. */
#include <cpuid.h>
#include <stdio.h>

int main(void) {
    unsigned a = 0, b = 0, c = 0, d = 0;
    unsigned b7 = 0;
    __get_cpuid(1, &a, &b, &c, &d);
    unsigned ecx1 = c;
    if (__get_cpuid_count(7, 0, &a, &b, &c, &d))
        b7 = b;
    printf("avx=%d avx2=%d avx512f=%d xsave=%d osxsave=%d\n",
           (ecx1 >> 28) & 1, (b7 >> 5) & 1, (b7 >> 16) & 1, (ecx1 >> 26) & 1,
           (ecx1 >> 27) & 1);
    fflush(stdout);
    __asm__ volatile("vaddps %%ymm1, %%ymm1, %%ymm0" ::: "xmm0");
    printf("vaddps ran\n");
    return 0;
}
