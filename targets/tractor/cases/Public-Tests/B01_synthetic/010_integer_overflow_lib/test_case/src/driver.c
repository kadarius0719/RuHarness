// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>

void printHexCharLine (char charHex)
{
    printf("%02x\n", charHex);
}

void driver(char data)
{
    char result = data + 1;
    printHexCharLine(result);
}