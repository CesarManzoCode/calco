/* The four signal cases of contract 6, compared byte for byte with the native run:
 *
 * 1. SIGSEGV writing to a read-only page: an SA_SIGINFO handler prints si_addr, the rip
 *    of the ucontext relative to the faulting instruction and four registers set right
 *    before, then moves rip past the instruction and the program goes on.
 * 2. SIGILL from ud2.
 * 3. SIGFPE from an integer division by zero.
 * 4. SIGALRM from setitimer in the middle of a register-only loop that keeps
 *    rax + rbx == 0x12345 and r12 == 0x5a5a5a5a: the handler prints whether rip is in the
 *    loop and whether the invariant holds (not the values, which depend on the moment),
 *    and ends the loop by setting r15 in the ucontext.
 *
 * Built with -O2 -static (non-PIE, contract D21); the labels mark the instructions. */
#define _GNU_SOURCE
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/time.h>
#include <ucontext.h>
#include <unistd.h>

extern char segv_at[], segv_end[], ill_at[], ill_end[], fpe_at[], fpe_end[];
extern char loop_start[], loop_end[];

static char *page;

static void say(const char *fmt, ...) {
    char buf[256];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    if (write(1, buf, n) != n)
        _exit(2);
}

static const char *name(int sig) {
    return sig == SIGSEGV ? "SIGSEGV" : sig == SIGILL ? "SIGILL" : sig == SIGFPE ? "SIGFPE" : "SIGALRM";
}

static void on_fault(int sig, siginfo_t *si, void *ctx) {
    greg_t *g = ((ucontext_t *)ctx)->uc_mcontext.gregs;
    char *at = sig == SIGSEGV ? segv_at : sig == SIGILL ? ill_at : fpe_at;
    char *end = sig == SIGSEGV ? segv_end : sig == SIGILL ? ill_end : fpe_end;
    /* si_addr is the data address for SIGSEGV and the instruction for the others. */
    char *base = sig == SIGSEGV ? page : at;
    say("%s: si_signo=%d si_code=%d si_addr=%+ld rip=%+ld trapno=%lld err=%#llx\n",
        name(sig), si->si_signo, si->si_code, (long)((char *)si->si_addr - base),
        (long)((char *)g[REG_RIP] - at), (long long)g[REG_TRAPNO], (long long)g[REG_ERR]);
    say("  rbx=%#llx r12=%#llx r13=%#llx r14=%#llx\n", (long long)g[REG_RBX],
        (long long)g[REG_R12], (long long)g[REG_R13], (long long)g[REG_R14]);
    g[REG_RIP] = (greg_t)end;
}

static void on_alarm(int sig, siginfo_t *si, void *ctx) {
    greg_t *g = ((ucontext_t *)ctx)->uc_mcontext.gregs;
    char *rip = (char *)g[REG_RIP];
    int in_loop = rip >= loop_start && rip < loop_end;
    int invariant = g[REG_RAX] + g[REG_RBX] == 0x12345 && g[REG_R12] == 0x5a5a5a5a;
    say("%s: si_signo=%d si_code=%d in_loop=%d invariant=%d\n", name(sig), si->si_signo,
        si->si_code, in_loop, invariant);
    g[REG_R15] = 1;
}

static void handle(int sig, void (*fn)(int, siginfo_t *, void *)) {
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = fn;
    sa.sa_flags = SA_SIGINFO;
    sigemptyset(&sa.sa_mask);
    if (sigaction(sig, &sa, NULL) != 0) {
        say("sigaction %d failed\n", sig);
        _exit(1);
    }
}

__attribute__((noipa)) static void segv_case(void) {
    __asm__ volatile("mov $0x1111111111111111, %%rbx\n\t"
                     "mov $0x2222, %%r12\n\t"
                     "mov $-3, %%r13\n\t"
                     "mov $0x4444444400000000, %%r14\n\t"
                     ".globl segv_at\n"
                     "segv_at:\n\t"
                     "movl $1, (%0)\n"
                     ".globl segv_end\n"
                     "segv_end:\n\t"
                     :
                     : "r"(page)
                     : "rbx", "r12", "r13", "r14", "memory");
}

__attribute__((noipa)) static void ill_case(void) {
    __asm__ volatile("mov $0x5, %%rbx\n\t"
                     "mov $0x6, %%r12\n\t"
                     "mov $0x7, %%r13\n\t"
                     "mov $0x8, %%r14\n\t"
                     ".globl ill_at\n"
                     "ill_at:\n\t"
                     "ud2\n"
                     ".globl ill_end\n"
                     "ill_end:\n\t"
                     :
                     :
                     : "rbx", "r12", "r13", "r14", "memory");
}

__attribute__((noipa)) static void fpe_case(void) {
    __asm__ volatile("mov $0xb0b0, %%rbx\n\t"
                     "mov $0xc0c0, %%r12\n\t"
                     "mov $0xd0d0, %%r13\n\t"
                     "mov $0xe0e0, %%r14\n\t"
                     "xor %%edx, %%edx\n\t"
                     "mov $7, %%eax\n\t"
                     "xor %%ecx, %%ecx\n"
                     ".globl fpe_at\n"
                     "fpe_at:\n\t"
                     "divl %%ecx\n"
                     ".globl fpe_end\n"
                     "fpe_end:\n\t"
                     :
                     :
                     : "rax", "rcx", "rdx", "rbx", "r12", "r13", "r14", "memory", "cc");
}

__attribute__((noipa)) static void alarm_case(void) {
    struct itimerval it;
    memset(&it, 0, sizeof it);
    it.it_value.tv_usec = 20000;
    if (setitimer(ITIMER_REAL, &it, NULL) != 0) {
        say("setitimer failed\n");
        _exit(1);
    }
    __asm__ volatile("mov $0x12000, %%rax\n\t"
                     "mov $0x345, %%rbx\n\t"
                     "mov $0x5a5a5a5a, %%r12\n\t"
                     "xor %%r15d, %%r15d\n\t"
                     "xor %%ecx, %%ecx\n"
                     ".globl loop_start\n"
                     "loop_start:\n\t"
                     "xchg %%rax, %%rbx\n\t"
                     "add $1, %%rcx\n\t"
                     "test %%r15, %%r15\n\t"
                     "jz loop_start\n"
                     ".globl loop_end\n"
                     "loop_end:\n\t"
                     :
                     :
                     : "rax", "rbx", "rcx", "r12", "r15", "memory", "cc");
}

int main(void) {
    page = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (page == MAP_FAILED) {
        say("mmap failed\n");
        return 1;
    }
    handle(SIGSEGV, on_fault);
    handle(SIGILL, on_fault);
    handle(SIGFPE, on_fault);
    handle(SIGALRM, on_alarm);

    segv_case();
    say("after SIGSEGV\n");
    ill_case();
    say("after SIGILL\n");
    fpe_case();
    say("after SIGFPE\n");
    alarm_case();
    say("after SIGALRM\n");
    return 0;
}
