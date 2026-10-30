/* Prints "ready" and blocks reading stdin, so noexec.py can read calco's
 * /proc/PID/maps while the guest is loaded and running. */
#include <stdio.h>
#include <unistd.h>

int main(void) {
    char c;
    printf("ready\n");
    fflush(stdout);
    if (read(0, &c, 1) < 0)
        return 1;
    return 0;
}
