// Linker hint file: references Rust FFI symbols to prevent dead-code stripping.
// This ensures dlsym(RTLD_DEFAULT, ...) can find flutter_rust_bridge symbols at runtime.
// Without this, symbols from libtorex_local_store.a are stripped by the linker
// because no Swift/ObjC code directly calls them.

extern void frb_get_rust_content_hash(void);

// Constructor ensures this function is always linked into the final binary,
// even if nothing directly references it. This forces the linker to resolve
// the reference to frb_get_rust_content_hash from the vendored static library.
__attribute__((constructor))
void _force_rust_symbols(void) {
    frb_get_rust_content_hash();
}
