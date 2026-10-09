#include "proj.h"

int main(void) {
    return decode("x") + (int)fast_crc("y");
}
