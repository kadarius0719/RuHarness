// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void printLine (const char * line)
{
    if(line != NULL) 
    {
        printf("%s\n", line);
    }
}

void driver(int data)
{
    char source[100];
    char dest[100] = "";
    memset(source, 'A', 100-1);
    source[100-1] = '\0';
    if (data < 100)
    {
        strncpy(dest, source, data);
        dest[data] = '\0';
    }
    printLine(dest);
}
