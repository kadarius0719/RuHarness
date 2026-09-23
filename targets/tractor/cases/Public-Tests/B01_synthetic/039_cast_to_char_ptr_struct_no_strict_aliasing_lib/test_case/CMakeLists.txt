# © 2026 Massachusetts Institute of Technology
# MIT License

cmake_minimum_required(VERSION 3.10)
project(driver)
add_library(driver SHARED
    src/driver.c)
target_compile_options(driver PRIVATE -fno-strict-aliasing)

target_include_directories(driver PUBLIC include)
target_include_directories(driver PRIVATE src)

install(TARGETS driver
    DESTINATION lib)
install(DIRECTORY include/
    DESTINATION include)
