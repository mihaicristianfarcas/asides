// A stand-in coding agent for the end-to-end test: built as `claude`, so asides finds it in the
// process table as Claude Code's session. Like a real one, it puts its terminal in raw mode and turns
// bracketed paste on; it logs every byte typed into it (escaped, one read per line) to the file
// $ASIDES_E2E_LOG, so the test sees exactly what asides typed.
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <termios.h>
#include <unistd.h>

int main(void) {
    const char *path = getenv("ASIDES_E2E_LOG");
    if (!path) return 2;
    struct termios raw;
    if (tcgetattr(0, &raw) == 0) {
        cfmakeraw(&raw);
        tcsetattr(0, TCSANOW, &raw);
    }
    // Bracketed paste on, and a prompt.
    const char *hello = "\x1b[?2004hasides e2e agent> ";
    write(1, hello, strlen(hello));
    unsigned char buf[4096];
    for (;;) {
        ssize_t n = read(0, buf, sizeof buf);
        if (n <= 0) return 0;
        FILE *log = fopen(path, "a");
        if (!log) return 3;
        for (ssize_t i = 0; i < n; i++) {
            unsigned char c = buf[i];
            if (c == 0x1b) fputs("<ESC>", log);
            else if (c == '\r') fputs("<CR>", log);
            else if (c == '\n') fputs("<LF>", log);
            else if (c == '\t') fputs("<TAB>", log);
            else if (c < 0x20) fprintf(log, "<%02x>", c);
            else fputc(c, log);
        }
        fputc('\n', log);
        fclose(log);
        // Ctrl-D ends it.
        if (memchr(buf, 4, n)) return 0;
    }
}
