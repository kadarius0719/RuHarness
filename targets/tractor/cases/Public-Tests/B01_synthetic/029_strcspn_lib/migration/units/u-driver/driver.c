#include "driver.h"

#include <stdio.h>

static void call_driver(int case_num, const char *s1, const char *s2) {
    printf("case %d driver(s1=\"%s\", s2=\"%s\")\n", case_num, s1, s2);
    driver(s1, s2);
}

int main(void) {
    int case_num = 1;

    /* Both empty: strcspn("", anything) is always 0. */
    call_driver(case_num++, "", "");
    call_driver(case_num++, "", "abc");

    /* Empty reject set: no character of s1 can match, so the whole of
       s1 is scanned and its full length is returned. */
    call_driver(case_num++, "abc", "");
    call_driver(case_num++, "a", "");
    call_driver(case_num++, "hello world", "");

    /* First character of s1 itself is in s2: result is 0. */
    call_driver(case_num++, "hello", "h");
    call_driver(case_num++, "hello", "oh");
    call_driver(case_num++, "a", "a");

    /* No character of s1 appears anywhere in s2: result is strlen(s1). */
    call_driver(case_num++, "hello", "xyz");
    call_driver(case_num++, "abcdef", "123456");

    /* Reject character appears in the middle: result is the offset of
       the first match. */
    call_driver(case_num++, "hello", "aeiou");
    call_driver(case_num++, "hello world", "wr");
    call_driver(case_num++, "mississippi", "sp");
    call_driver(case_num++, "abcdefg", "dz");

    /* Reject character appears only at the very end of s1: result is
       strlen(s1) - 1. */
    call_driver(case_num++, "abcdefz", "z");
    call_driver(case_num++, "single", "e");

    /* Reject set has duplicate/overlapping characters and characters
       not present in s1 at all. */
    call_driver(case_num++, "abcabc", "aa");
    call_driver(case_num++, "banana", "nqrz");

    /* Single-character s1 and s2 combinations, matching and not. */
    call_driver(case_num++, "x", "y");
    call_driver(case_num++, "x", "x");

    /* Whitespace and punctuation as reject characters. */
    call_driver(case_num++, "one two three", " ");
    call_driver(case_num++, "key=value;next=1", "=;");
    call_driver(case_num++, "no-separators-here", " \t\n");

    /* s1 made entirely of characters that are all in s2: result is 0
       immediately (also exercises the case where every character of a
       longer buffer is a reject character, a "full buffer" match). */
    call_driver(case_num++, "aaaaaaaaaa", "a");
    call_driver(case_num++, "abcabcabcabc", "cba");

    /* Longer strings to exercise repeated scanning. */
    call_driver(case_num++, "the quick brown fox jumps over the lazy dog", "aeiou");
    call_driver(case_num++, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "b");
    call_driver(case_num++, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "b");

    return 0;
}
