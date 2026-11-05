// A stand-in coding agent for the end-to-end test: built as `claude`, so asides finds it in the
// process table as Claude Code's session. Like a real one, it puts its terminal in raw mode and turns
// bracketed paste on; it logs every byte typed into it (escaped, one read per line) to the file
// $ASIDES_E2E_LOG, so the test sees exactly what asides typed. On Windows, its console reads as
// Node's does in raw mode: keys as the characters (and escape sequences) they type, in UTF-16.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <fcntl.h>
#include <termios.h>
#include <unistd.h>
#endif

static void raw_mode(void) {
#ifdef _WIN32
    HANDLE in = GetStdHandle(STD_INPUT_HANDLE), out = GetStdHandle(STD_OUTPUT_HANDLE);
    SetConsoleMode(in, ENABLE_VIRTUAL_TERMINAL_INPUT);
    DWORD mode = 0;
    GetConsoleMode(out, &mode);
    SetConsoleMode(out, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
#else
    struct termios raw;
    if (tcgetattr(0, &raw) == 0) {
        cfmakeraw(&raw);
        tcsetattr(0, TCSANOW, &raw);
    }
#endif
}

static long read_input(unsigned char *buf, size_t size) {
#ifdef _WIN32
    // As UTF-16, as Node, Bun and crossterm read a console, then as UTF-8.
    wchar_t wide[1024];
    DWORD n = 0;
    if (!ReadConsoleW(GetStdHandle(STD_INPUT_HANDLE), wide, 1024, &n, NULL)) return -1;
    return WideCharToMultiByte(CP_UTF8, 0, wide, (int)n, (char *)buf, (int)size, NULL, NULL);
#else
    return (long)read(0, buf, size);
#endif
}

int main(void) {
    const char *path = getenv("ASIDES_E2E_LOG");
    if (!path) return 2;
    raw_mode();
    // Bracketed paste on, and a prompt.
    const char *hello = "\x1b[?2004hasides e2e agent> ";
    fwrite(hello, 1, strlen(hello), stdout);
    fflush(stdout);
    unsigned char buf[4096];
    for (;;) {
        long n = read_input(buf, sizeof buf);
        if (n <= 0) return 0;
        FILE *log = fopen(path, "a");
        if (!log) return 3;
        for (long i = 0; i < n; i++) {
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
        if (memchr(buf, 4, (size_t)n)) return 0;
    }
}
