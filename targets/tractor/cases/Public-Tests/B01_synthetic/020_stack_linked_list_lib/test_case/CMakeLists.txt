# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(SimpleList)
add_library(SimpleList SHARED
    src/simplestruct.c)

target_include_directories(SimpleList PUBLIC include)
target_include_directories(SimpleList PRIVATE src)

install(TARGETS SimpleList
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
