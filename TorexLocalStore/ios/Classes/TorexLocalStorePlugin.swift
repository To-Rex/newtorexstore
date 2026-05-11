import Flutter

// Declare the rust content hash function with its symbol name.
// This creates a reference that forces the linker to include the
// symbol from the vendored static library, making it available
// via dlsym(RTLD_DEFAULT, ...) for flutter_rust_bridge.
@_silgen_name("frb_get_rust_content_hash")
func frb_get_rust_content_hash() -> Int

public class TorexLocalStorePlugin: NSObject, FlutterPlugin {
    public static func register(with registrar: FlutterPluginRegistrar) {
        // Trigger Rust symbol linkage on first plugin registration
        let _ = frb_get_rust_content_hash()
    }
}
