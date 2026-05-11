import FlutterMacOS

@_silgen_name("frb_get_rust_content_hash")
func frb_get_rust_content_hash() -> Int

public class TorexLocalStorePlugin: NSObject, FlutterPlugin {
    public static func register(with registrar: FlutterPluginRegistrar) {
        let _ = frb_get_rust_content_hash()
    }
}
