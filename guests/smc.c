/* Self-modifying code of contract 3.13, compared byte for byte with the native run (k2-04).
 *
 * - A RWX page gets `mov eax, imm32; ret`; it is called, the immediate is rewritten and
 *   it is called again, a few times.
 * - A store to a data byte of the same page between calls, from code in another page.
 * - A function that, with a RIP-relative `mov byte`, rewrites the immediate of the `mov`
 *   that follows it in the same block, and returns it.
 * - mprotect to read and execute, a call, mprotect back to RWX, a rewrite and a call.
 * - The page is unmapped and mapped again at the same address with other code; then
 *   madvise(MADV_DONTNEED) zeroes it and new code goes in; then mremap moves it, and the
 *   old address gets fresh code with a fixed mmap.
 *
 * Each call prints what it returned. Built with -O2 -static. */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

typedef int (*fn)(void);

static volatile unsigned char *page;
static size_t page_size = 4096;

/* `mov eax, imm; ret` at offset `at`. */
static void put_ret(size_t at, uint32_t imm) {
    page[at] = 0xb8;
    for (int i = 0; i < 4; i++)
        page[at + 1 + i] = (unsigned char)(imm >> (8 * i));
    page[at + 5] = 0xc3;
}

static void set_imm(size_t at, uint32_t imm) {
    for (int i = 0; i < 4; i++)
        page[at + 1 + i] = (unsigned char)(imm >> (8 * i));
}

static int call(size_t at) {
    __builtin___clear_cache((char *)page + at, (char *)page + at + 64);
    return ((fn)(uintptr_t)(page + at))();
}

static void fail(const char *what) {
    perror(what);
    _exit(1);
}

int main(void) {
    void *p = mmap(NULL, page_size, PROT_READ | PROT_WRITE | PROT_EXEC,
                   MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED)
        fail("mmap");
    page = p;

    /* Rewrite the immediate between calls. */
    put_ret(0, 1);
    printf("first: %d\n", call(0));
    set_imm(0, 2);
    printf("rewritten: %d\n", call(0));
    for (uint32_t i = 3; i < 8; i++) {
        set_imm(0, i * 100);
        printf("again: %d\n", call(0));
    }

    /* A data byte in the code page: the code stays as it is. */
    page[0x800] = 0x5a;
    printf("after data store: %d (data %d)\n", call(0), page[0x800]);
    page[0x800]++;
    printf("after data store: %d (data %d)\n", call(0), page[0x800]);

    /* mov byte [rip + 1], 42  (C6 05 disp32 imm8): the next instruction's immediate. */
    static const unsigned char self[] = {
        0xc6, 0x05, 0x01, 0x00, 0x00, 0x00, 0x2a, /* mov byte [rip+1], 42 */
        0xb8, 0x01, 0x00, 0x00, 0x00,             /* mov eax, 1 */
        0xc3,                                     /* ret */
    };
    const size_t at = 0x100;
    for (size_t i = 0; i < sizeof self; i++)
        page[at + i] = self[i];
    printf("self-modified: %d\n", call(at));
    printf("self-modified again: %d\n", call(at));
    page[at + 6] = 7;  /* the value it stores */
    page[at + 8] = 1;  /* the immediate it rewrites */
    printf("self-modified with 7: %d\n", call(at));

    /* Read-only and back. */
    if (mprotect(p, page_size, PROT_READ | PROT_EXEC) != 0)
        fail("mprotect r-x");
    printf("read-only: %d\n", call(0));
    if (mprotect(p, page_size, PROT_READ | PROT_WRITE | PROT_EXEC) != 0)
        fail("mprotect rwx");
    set_imm(0, 1234);
    printf("writable again: %d\n", call(0));

    /* munmap and a new mapping at the same address. */
    if (munmap(p, page_size) != 0)
        fail("munmap");
    void *q = mmap(p, page_size, PROT_READ | PROT_WRITE | PROT_EXEC,
                   MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    if (q != p)
        fail("mmap again");
    put_ret(0, 5555);
    printf("remapped: %d\n", call(0));

    /* MADV_DONTNEED zeroes a private anonymous page. */
    if (madvise(p, page_size, MADV_DONTNEED) != 0)
        fail("madvise");
    printf("after madvise: byte %d\n", page[0]);
    put_ret(0, 6666);
    printf("after madvise: %d\n", call(0));

    /* mremap moves the page; fresh code at the old address. */
    void *spot = mmap(NULL, page_size, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (spot == MAP_FAILED)
        fail("mmap spot");
    void *moved = mremap(p, page_size, page_size, MREMAP_MAYMOVE | MREMAP_FIXED, spot);
    if (moved != spot)
        fail("mremap");
    page = moved;
    printf("moved: %d\n", call(0));
    set_imm(0, 7777);
    printf("moved and rewritten: %d\n", call(0));
    q = mmap(p, page_size, PROT_READ | PROT_WRITE | PROT_EXEC,
             MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    if (q != p)
        fail("mmap old");
    page = q;
    put_ret(0, 8888);
    printf("old address: %d\n", call(0));
    return 0;
}
