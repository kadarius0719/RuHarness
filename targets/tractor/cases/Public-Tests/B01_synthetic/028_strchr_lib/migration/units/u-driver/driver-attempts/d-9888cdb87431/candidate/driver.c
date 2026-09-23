#include "driver.h"

#include <stdio.h>

extern int foo(const char *in, char c);

/* foo() scans with `for (const char *s = in; s = strchr(s, c); s++)`.
   When c is the string's own terminating '\0', strchr matches the
   terminator itself, the loop body runs once, and then s++ steps one
   byte past the terminator, i.e. one byte past the end of whatever
   object holds the string; the next strchr call on that dangling
   pointer reads out of bounds no matter how the buffer is sized, since
   the buffer is always finite and the walk never stops once c == '\0'.
   That out-of-bounds read is the unit's own latent bug, not something
   this driver may trigger, so every call below uses a non-NUL c. */

static void call_foo(int case_num, const char *in, char c) {
    printf("case %d foo(in=\"%s\", c=%d) ret=%d\n", case_num, in, (int)c, foo(in, c));
}

static void call_driver(int case_num, const char *in) {
    printf("case %d driver(in=\"%s\")\n", case_num, in);
    driver(in);
}

int main(void) {
    int case_num = 1;

    call_foo(case_num++, "", 'A');
    call_foo(case_num++, "", 'x');
    call_foo(case_num++, "", 'z');
    call_foo(case_num++, "A", 'A');
    call_foo(case_num++, "x", 'x');
    call_foo(case_num++, "Ax", 'A');
    call_foo(case_num++, "Ax", 'x');
    call_foo(case_num++, "xA", 'A');
    call_foo(case_num++, "AAAA", 'A');
    call_foo(case_num++, "xxxxxxxxxx", 'x');
    call_foo(case_num++, "AxAxAxAxAx", 'A');
    call_foo(case_num++, "AxAxAxAxAx", 'x');
    call_foo(case_num++, "banana", 'a');
    call_foo(case_num++, "banana", 'n');
    call_foo(case_num++, "banana", 'z');
    call_foo(case_num++, "Mississippi", 's');
    call_foo(case_num++, "Mississippi", 'i');
    call_foo(case_num++, "Mississippi", 'p');
    call_foo(case_num++, "Mississippi", 'M');
    call_foo(case_num++, "The Quick Brown Fox", 'o');
    call_foo(case_num++, "The Quick Brown Fox", 'Q');
    call_foo(case_num++, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", 'A');
    call_foo(case_num++, "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB", 'A');
    call_foo(case_num++, "edge\x01" "case\x01here", '\x01');
    call_foo(case_num++, "tail-A", 'A');
    call_foo(case_num++, "A-head", 'A');
    call_foo(case_num++, "mixed\x7F" "bytes\x7F", '\x7F');
    call_foo(case_num++, "high\xFF" "bytes\xFF", '\xFF');
    call_foo(case_num++, "single", 'g');
    call_foo(case_num++, "aAaAaA", 'A');

    call_driver(case_num++, "");
    call_driver(case_num++, "A");
    call_driver(case_num++, "x");
    call_driver(case_num++, "Ax");
    call_driver(case_num++, "xA");
    call_driver(case_num++, "AAAA");
    call_driver(case_num++, "xxxx");
    call_driver(case_num++, "AxAxAx");
    call_driver(case_num++, "banana");
    call_driver(case_num++, "apple");
    call_driver(case_num++, "Mississippi");
    call_driver(case_num++, "Fox jumps over");
    call_driver(case_num++, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    call_driver(case_num++, "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
    call_driver(case_num++, "AxAxAxAxAxAxAxAxAxAxAxAxAxAxAxAxAxAxAxAx");
    call_driver(case_num++, "A\tx\nA");
    call_driver(case_num++, "high\xFF" "byteA\xFF" "x");

    return 0;
}
