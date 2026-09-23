// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <math.h>
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

void bad(float data)
{
    int result = (int)(100.0 / data);
    printIntLine(result);
}

static void goodG2B()
{
    float data;
    data = 2.0F;
    {
        int result = (int)(100.0 / data);
        printIntLine(result);
    }
}

static void goodB2G(float data)
{
    if (fabs(data) > 0.000001)
    {
        int result = (int)(100.0 / data);
        printIntLine(result);
    }
    else
    {
        printLine("This would result in a divide by zero");
    }
}

void good(float data)
{
    goodG2B();
    goodB2G(data);
}

void driver(float goodData, float badData)
{
    printLine("Calling good()...");
    good(goodData);
    printLine("Finished good()");
    printLine("Calling bad()...");
    bad(badData);
    printLine("Finished bad()");
}
