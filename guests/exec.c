/* The parent prints the AVX bit of CPUID and forks; the child execve's this same program
 * through /proc/self/exe with the argument "child", prints the AVX bit again and exits
 * with 7. Under calco both bits are 0: the new program also runs translated
 * (guests/exec.expected, tests/exec.rs). */
#include <cpuid.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

static int avx(void) {
    unsigned a = 0, b = 0, c = 0, d = 0;
    __get_cpuid(1, &a, &b, &c, &d);
    return (c >> 28) & 1;
}

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "child") == 0) {
        printf("child avx=%d\n", avx());
        return 7;
    }
    printf("parent avx=%d\n", avx());
    fflush(stdout);
    pid_t pid = fork();
    if (pid < 0) {
        perror("fork");
        return 1;
    }
    if (pid == 0) {
        char *args[] = {argv[0], "child", NULL};
        execve("/proc/self/exe", args, environ);
        perror("execve");
        _exit(1);
    }
    int status;
    if (waitpid(pid, &status, 0) != pid) {
        perror("waitpid");
        return 1;
    }
    if (WIFEXITED(status))
        printf("child exit=%d\n", WEXITSTATUS(status));
    else
        printf("child signal=%d\n", WTERMSIG(status));
    return 0;
}
