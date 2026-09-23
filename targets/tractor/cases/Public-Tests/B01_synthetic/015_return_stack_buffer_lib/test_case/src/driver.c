// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>

void printLine(const char *line)
{
    if (line != NULL)
    {
        printf("%s\n", line);
    }
}

static char *helperBad()
{
    char charString[] = "helperBad string";
    return charString;
}

void bad() 
{
    printLine(helperBad());
}

static char *helperGood1()
{
    static char charString[] = "helperGood1 string";
    return charString;
}

void good() 
{
    printLine(helperGood1());
}

void driver(int useGood)
{
    if (useGood)
    {
        good();
    }
    else
    {
        bad();
    }
}
