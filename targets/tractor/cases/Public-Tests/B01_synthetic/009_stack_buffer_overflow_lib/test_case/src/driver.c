// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

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

void bad(int data)
{
    int i;
    int buffer[10] = { 0 };
    if (data >= 0)
    {
        buffer[data] = 1;
        /* Print the array values */
        for(i = 0; i < 10; i++)
        {
            printIntLine(buffer[i]);
        }
    }
    else
    {
        printLine("ERROR: Array index is negative.");
    }
}

static void goodG2B()
{
    int data = 7;
    int i;
    int buffer[10] = { 0 };
    if (data >= 0)
    {
        buffer[data] = 1;
        /* Print the array values */
        for(i = 0; i < 10; i++)
        {
            printIntLine(buffer[i]);
        }
    }
    else
    {
        printLine("ERROR: Array index is negative.");
    }
}

static void goodB2G(int data)
{
    int i;
    int buffer[10] = { 0 };
    if (data >= 0 && data < (10))
    {
        buffer[data] = 1;
        /* Print the array values */
        for(i = 0; i < 10; i++)
        {
            printIntLine(buffer[i]);
        }
    }
    else
    {
        printLine("ERROR: Array index is out-of-bounds");
    }
}

void good(int data)
{
    goodG2B();
    goodB2G(data);
}

void driver(int goodData, int badData)
{
    printLine("Calling good()...");
    good(goodData);
    printLine("Finished good()");
    printLine("Calling bad()...");
    bad(badData);
    printLine("Finished bad()");
}
