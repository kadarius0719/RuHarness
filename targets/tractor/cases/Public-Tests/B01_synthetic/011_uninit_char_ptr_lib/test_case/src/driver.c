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

void bad()
{
    char *data;
    printLine(data);
}

void good()
{
    char *data;
    data = "string";
    printLine(data);
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
