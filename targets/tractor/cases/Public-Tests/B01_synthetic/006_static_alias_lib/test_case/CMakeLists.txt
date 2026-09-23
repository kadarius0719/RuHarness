# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(StaticAlias)
add_library(StaticAlias SHARED
    src/staticalias.c)

target_include_directories(StaticAlias PUBLIC include)
target_include_directories(StaticAlias PRIVATE src)

install(TARGETS StaticAlias
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
