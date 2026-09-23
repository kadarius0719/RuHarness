# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(Loop)
add_library(Loop SHARED
    src/loop.c)

target_include_directories(Loop PUBLIC include)
target_include_directories(Loop PRIVATE src)

install(TARGETS Loop
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
