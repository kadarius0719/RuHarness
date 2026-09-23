# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(echo)
add_library(echo SHARED
    src/echo.c)

target_include_directories(echo PUBLIC include)
target_include_directories(echo PRIVATE src)

install(TARGETS echo
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
