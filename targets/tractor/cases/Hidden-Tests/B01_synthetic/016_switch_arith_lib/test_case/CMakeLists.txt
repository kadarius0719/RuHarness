# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(switch-arith)
add_library(switch-arith SHARED
    src/switch-arith.c)

target_include_directories(switch-arith PUBLIC include)
target_include_directories(switch-arith PRIVATE src)

install(TARGETS switch-arith
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
