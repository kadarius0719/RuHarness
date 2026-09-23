# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(hello)
add_library(hello SHARED
    src/hello.c)

target_include_directories(hello PUBLIC include)
target_include_directories(hello PRIVATE src)

install(TARGETS hello
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
