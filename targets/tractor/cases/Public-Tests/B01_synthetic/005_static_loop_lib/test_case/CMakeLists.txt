# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(StaticLoop)
add_library(StaticLoop SHARED
    src/staticloop.c)

target_include_directories(StaticLoop PUBLIC include)
target_include_directories(StaticLoop PRIVATE src)

install(TARGETS StaticLoop
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
