# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(pow)
add_library(pow SHARED
    src/pow.c)

target_include_directories(pow PUBLIC include)
target_include_directories(pow PRIVATE src)

# Ensure that shared library is linked with libm math lib
# "#include <math.h>" requires that compile links with -lm
target_link_libraries(pow m)

install(TARGETS pow
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
