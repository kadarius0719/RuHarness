// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <alloca.h>
#include <stdio.h>
#include <stdlib.h>

void printLine (const char * line)
{
    if(line != NULL) 
    {
        printf("%s\n", line);
    }
}

void printIntLine (int intNumber)
{
    printf("%d\n", intNumber);
}

void bad()
{
    int * data;
    data = (int *)alloca(10);
    {
        int source[10] = {0};
        size_t i;
        for (i = 0; i < 10; i++)
        {
            data[i] = source[i];
        }
        printIntLine(data[0]);
    }
}

void good()
{
    int * data;
    data = NULL;
    data = (int *)alloca(10*sizeof(int));
    {
        int source[10] = {0};
        size_t i;
        for (i = 0; i < 10; i++)
        {
            data[i] = source[i];
        }
        printIntLine(data[0]);
    }
}

void driver(int useGood) {
    if (useGood)
    {
        good();
    }
    else
    {
        bad();
    }
}
