# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(String_Slice)
add_library(String_Slice SHARED
    src/slicing.c)

target_include_directories(String_Slice PUBLIC include)
target_include_directories(String_Slice PRIVATE src)

install(TARGETS String_Slice
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
