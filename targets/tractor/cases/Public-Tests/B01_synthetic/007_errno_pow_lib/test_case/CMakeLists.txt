# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(pow)
add_library(pow SHARED
    src/pow.c)

target_link_libraries(pow m)
target_include_directories(pow PUBLIC include)
target_include_directories(pow PRIVATE src)

install(TARGETS pow
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
