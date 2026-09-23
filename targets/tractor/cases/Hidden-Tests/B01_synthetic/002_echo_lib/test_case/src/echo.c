// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>

#include "echo.h"

/*
Print out args one by one.
*/
int echo(int argc, char **argv) {

    for (int i = 1; i < argc; i++) {
        printf("%s\n", argv[i]);
    }

    return 0;
}
