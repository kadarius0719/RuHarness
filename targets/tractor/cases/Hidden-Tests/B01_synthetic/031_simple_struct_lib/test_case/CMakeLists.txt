# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(SimpleStruct)
add_library(SimpleStruct SHARED
    src/simplestruct.c)

target_include_directories(SimpleStruct PUBLIC include)
target_include_directories(SimpleStruct PRIVATE src)

install(TARGETS SimpleStruct
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
