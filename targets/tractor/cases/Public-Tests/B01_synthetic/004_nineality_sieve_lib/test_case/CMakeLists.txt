# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(Sieve)
add_library(Sieve SHARED
    src/sieve.c)

target_include_directories(Sieve PUBLIC include)
target_include_directories(Sieve PRIVATE src)

install(TARGETS Sieve
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
