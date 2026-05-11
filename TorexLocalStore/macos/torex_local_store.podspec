Pod::Spec.new do |s|
  s.name             = 'torex_local_store'
  s.version          = '0.1.0'
  s.summary          = 'Ultra-high-performance local storage engine powered by Rust'
  s.description      = <<-DESC
TorexLocalStore is an ultra-high-performance local storage engine for Flutter,
powered by Rust. It uses LSM-tree architecture with memory-mapped files,
zero-copy reads, WAL crash recovery, and reactive streams.
                       DESC
  s.homepage         = 'https://github.com/torex/torexstore'
  s.license          = { :type => 'MIT', :file => '../LICENSE' }
  s.author           = { 'Torex' => 'dev@torex.uz' }
  s.source           = { :git => 'https://github.com/torex/torexstore.git', :tag => s.version.to_s }

  s.platform         = :osx, '10.14'
  s.swift_version    = '5.0'

  s.static_framework = true

  # Source files - Swift plugin class only
  s.source_files = 'Classes/**/*.swift'

  # Pre-built Rust static library (universal: arm64 + x86_64)
  s.vendored_libraries = 'Classes/libtorex_local_store.a'

  # System frameworks needed by Rust stdlib
  s.frameworks = 'Foundation', 'Security', 'SystemConfiguration'

  # Force load the static library to ensure all FFI symbols are available
  s.pod_target_xcconfig = {
    'OTHER_LDFLAGS' => '-force_load $(PODS_TARGET_SRCROOT)/Classes/libtorex_local_store.a'
  }

  s.dependency 'FlutterMacOS'
end
