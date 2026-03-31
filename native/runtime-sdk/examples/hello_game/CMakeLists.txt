cmake_minimum_required(VERSION 3.21)
project(hello_game LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_VISIBILITY_PRESET hidden)
set(CMAKE_VISIBILITY_INLINES_HIDDEN YES)

add_library(hello_game SHARED src/game.cpp)
target_include_directories(
    hello_game
    PRIVATE
        ${CMAKE_CURRENT_SOURCE_DIR}/../../include
)

if(MSVC)
    target_compile_definitions(hello_game PRIVATE _CRT_SECURE_NO_WARNINGS)
endif()
