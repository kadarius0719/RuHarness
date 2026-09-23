# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(StaticDag)
add_library(StaticDag SHARED
    src/staticdag.c)

target_include_directories(StaticDag PUBLIC include)
target_include_directories(StaticDag PRIVATE src)

install(TARGETS StaticDag
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
