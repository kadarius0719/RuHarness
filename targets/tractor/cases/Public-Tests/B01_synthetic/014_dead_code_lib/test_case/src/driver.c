// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>
#include <stdlib.h>

void printLine(const char *line)
{
    if (line != NULL)
    {
        printf("%s\n", line);
    }
}

static void helperBad()
{
    printLine("helperBad()");
}

void bad()
{
    printLine("bad()");
}

static void helperGood()
{
    printLine("helperGood()");
}

void good()
{
    printLine("good()");
    helperGood();
}

void driver()
{
    printLine("Calling good()...");
    good();
    printLine("Finished good()");
    printLine("Calling bad()...");
    bad();
    printLine("Finished bad()");
}
