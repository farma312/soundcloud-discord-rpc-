cmake_minimum_required(VERSION 3.21)
project(zen_beta_hud LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(DEFINED ENV{WEBVIEW2_SDK_DIR})
    set(WEBVIEW2_SDK_DIR "$ENV{WEBVIEW2_SDK_DIR}")
else()
    set(WEBVIEW2_SDK_DIR "${CMAKE_CURRENT_SOURCE_DIR}/webview2_sdk")
endif()
if(NOT EXISTS "${WEBVIEW2_SDK_DIR}/build/native/include/WebView2.h")
    message(FATAL_ERROR "WebView2 SDK not found. Set WEBVIEW2_SDK_DIR or place it in cpp_hud/webview2_sdk")
endif()
set(WEBVIEW2_INCLUDE "${WEBVIEW2_SDK_DIR}/build/native/include")
set(WEBVIEW2_LIB "${WEBVIEW2_SDK_DIR}/build/native/x64")

add_executable(zen_beta_hud WIN32 main.cpp)
target_include_directories(zen_beta_hud PRIVATE "${WEBVIEW2_INCLUDE}")
target_link_directories(zen_beta_hud PRIVATE "${WEBVIEW2_LIB}")
target_link_libraries(zen_beta_hud PRIVATE WebView2Loader.dll.lib user32 ole32 shlwapi)
target_compile_definitions(zen_beta_hud PRIVATE UNICODE _UNICODE NOMINMAX)

add_custom_command(TARGET zen_beta_hud POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${WEBVIEW2_LIB}/WebView2Loader.dll"
        "$<TARGET_FILE_DIR:zen_beta_hud>/WebView2Loader.dll")
