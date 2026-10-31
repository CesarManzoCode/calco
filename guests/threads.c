/* Threads of contract 6, compared byte for byte with the native run (k2-03).
 *
 * 8 threads start together at a gate built with pthread_mutex and pthread_cond. Each one
 * mixes a xorshift sequence seeded by its index into a __thread accumulator, adds 1 to a
 * C11 atomic counter on every step and, every 64 steps, adds to a sum under the mutex.
 * Then they take turns in index order through a condition variable, and return
 * index * index to pthread_join.
 *
 * Signals: while every thread waits at the gate, SIGUSR1 goes to thread 5 with
 * pthread_kill (tgkill), and SIGUSR2 to the process with kill while only thread 2 leaves
 * it unblocked. The handlers note which thread ran them through a __thread index.
 *
 * Only main prints, after joining, in index order. Built with -O2 -static -pthread. */
#define _GNU_SOURCE
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdio.h>
#include <unistd.h>

#define N 8
#define STEPS 200000
#define MAIN N

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t gate = PTHREAD_COND_INITIALIZER;
static pthread_cond_t turns = PTHREAD_COND_INITIALIZER;
static int arrived, gate_open, turn;
static char order[N + 1];
static unsigned long locked_sum;
static atomic_ulong counter;

static __thread unsigned long acc = 7;
static __thread int self = MAIN;

static volatile sig_atomic_t usr1_by[N + 1], usr2_by[N + 1];
static unsigned long first_acc[N], last_acc[N], result[N];
static int first_self[N];

static void on_usr1(int sig) {
    (void)sig;
    usr1_by[self]++;
}

static void on_usr2(int sig) {
    (void)sig;
    usr2_by[self]++;
}

static void *worker(void *arg) {
    int idx = (int)(long)arg;
    first_acc[idx] = acc;
    first_self[idx] = self;
    self = idx;
    if (idx == 2) {
        sigset_t s;
        sigemptyset(&s);
        sigaddset(&s, SIGUSR2);
        pthread_sigmask(SIG_UNBLOCK, &s, NULL);
    }

    pthread_mutex_lock(&lock);
    arrived++;
    pthread_cond_broadcast(&gate);
    while (!gate_open)
        pthread_cond_wait(&gate, &lock);
    pthread_mutex_unlock(&lock);

    unsigned long x = 0x9e3779b97f4a7c15ul * (unsigned long)(idx + 1);
    for (long i = 0; i < STEPS; i++) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        acc = acc * 31 + (x & 0xffff);
        atomic_fetch_add(&counter, 1);
        if ((i & 63) == 0) {
            pthread_mutex_lock(&lock);
            locked_sum += x & 0xff;
            pthread_mutex_unlock(&lock);
        }
    }
    result[idx] = x;
    last_acc[idx] = acc;

    pthread_mutex_lock(&lock);
    while (turn != idx)
        pthread_cond_wait(&turns, &lock);
    order[idx] = (char)('0' + idx);
    turn++;
    pthread_cond_broadcast(&turns);
    pthread_mutex_unlock(&lock);
    return (void *)(long)(idx * idx);
}

int main(void) {
    struct sigaction sa = {0};
    sa.sa_handler = on_usr1;
    sigaction(SIGUSR1, &sa, NULL);
    sa.sa_handler = on_usr2;
    sigaction(SIGUSR2, &sa, NULL);
    /* Inherited by every thread; thread 2 unblocks it. */
    sigset_t s;
    sigemptyset(&s);
    sigaddset(&s, SIGUSR2);
    pthread_sigmask(SIG_BLOCK, &s, NULL);
    acc = 12345;

    pthread_t t[N];
    for (long i = 0; i < N; i++) {
        if (pthread_create(&t[i], NULL, worker, (void *)i) != 0) {
            fprintf(stderr, "pthread_create %ld failed\n", i);
            return 1;
        }
    }

    pthread_mutex_lock(&lock);
    while (arrived < N)
        pthread_cond_wait(&gate, &lock);
    pthread_mutex_unlock(&lock);
    /* Every thread is at the gate, and thread 2 has SIGUSR2 unblocked. */
    pthread_kill(t[5], SIGUSR1);
    kill(getpid(), SIGUSR2);
    pthread_mutex_lock(&lock);
    gate_open = 1;
    pthread_cond_broadcast(&gate);
    pthread_mutex_unlock(&lock);

    long joined[N];
    for (int i = 0; i < N; i++) {
        void *r;
        pthread_join(t[i], &r);
        joined[i] = (long)r;
    }

    for (int i = 0; i < N; i++)
        printf("thread %d: tls start %lu/%d, result %016lx, tls %016lx, joined %ld, "
               "usr1 %d, usr2 %d\n",
               i, first_acc[i], first_self[i], result[i], last_acc[i], joined[i],
               (int)usr1_by[i], (int)usr2_by[i]);
    printf("main: tls %lu, usr1 %d, usr2 %d\n", acc, (int)usr1_by[MAIN], (int)usr2_by[MAIN]);
    printf("order: %s\n", order);
    printf("counter: %lu\n", (unsigned long)atomic_load(&counter));
    printf("locked sum: %lu\n", locked_sum);
    return 0;
}
