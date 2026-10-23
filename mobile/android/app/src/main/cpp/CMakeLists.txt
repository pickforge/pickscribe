cmake_minimum_required(VERSION 3.22.1)
project(pickscribe_whisper LANGUAGES C CXX)

# whisper.cpp at -O0 takes minutes per sentence, so debug app builds get optimized native code too.
set(CMAKE_BUILD_TYPE Release)

include(FetchContent)

# The same whisper.cpp release that the desktop gets through whisper-rs-sys 0.15.
FetchContent_Declare(
    whisper
    URL https://github.com/ggml-org/whisper.cpp/archive/refs/tags/v1.8.3.tar.gz
    URL_HASH SHA256=870ba21409cdf66697dc4db15ebdb13bc67037d76c7cc63756c81471d8f1731a
    # Lets whisper_full reuse the encoding that language detection made, instead of encoding
    # the same audio twice. The patch step runs only on a fresh download, so delete
    # android/app/.cxx after editing the patch.
    PATCH_COMMAND ${CMAKE_COMMAND} -DSOURCE=<SOURCE_DIR>
        -DPATCH=${CMAKE_CURRENT_SOURCE_DIR}/whisper-reuse-encoding.patch
        -P ${CMAKE_CURRENT_SOURCE_DIR}/patch-whisper.cmake
)

set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(WHISPER_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(WHISPER_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(WHISPER_BUILD_SERVER OFF CACHE BOOL "" FORCE)
set(GGML_NATIVE OFF CACHE BOOL "" FORCE)
# The NDK ships OpenMP only as a shared library; ggml's own thread pool avoids packaging it.
set(GGML_OPENMP OFF CACHE BOOL "" FORCE)
if(ANDROID_ABI STREQUAL "arm64-v8a")
    set(GGML_CPU_ARM_ARCH "armv8.2-a+dotprod+fp16" CACHE STRING "" FORCE)
endif()

FetchContent_MakeAvailable(whisper)

add_library(pickscribe_whisper SHARED pickscribe_whisper.cpp)
target_link_libraries(pickscribe_whisper PRIVATE whisper)
target_compile_options(pickscribe_whisper PRIVATE -O3 -fvisibility=hidden)
target_link_options(pickscribe_whisper PRIVATE -Wl,--gc-sections -Wl,-z,max-page-size=16384)
