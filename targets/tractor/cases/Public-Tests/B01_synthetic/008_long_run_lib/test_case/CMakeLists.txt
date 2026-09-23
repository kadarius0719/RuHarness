# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(long)
add_library(long SHARED
    src/long.c)

target_include_directories(long PUBLIC include)
target_include_directories(long PRIVATE src)

install(TARGETS long
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
