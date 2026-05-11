cmake_minimum_required(VERSION 3.15)
project(torex_local_store LANGUAGES CXX)

# This CMakeLists.txt is used when the package is consumed as a Flutter plugin
# on Windows. It links the pre-built Rust static library.

set(TOREX_LIB_DIR "${CMAKE_CURRENT_SOURCE_DIR}/lib")

# Check if pre-built library exists
if(EXISTS "${TOREX_LIB_DIR}/torex_local_store.lib")
    message(STATUS "TorexLocalStore: Found pre-built Rust static library")

    # Create a static imported library target
    add_library(torex_rust STATIC IMPORTED)
    set_target_properties(torex_rust PROPERTIES
        IMPORTED_LOCATION "${TOREX_LIB_DIR}/torex_local_store.lib"
    )

    # System libraries needed by Rust stdlib on Windows
    target_link_libraries(torex_rust INTERFACE
        ws2_32
        userenv
        ntdll
        bcrypt
    )
else()
    message(WARNING "TorexLocalStore: Pre-built Rust library not found at ${TOREX_LIB_DIR}/torex_local_store.lib")
endif()
