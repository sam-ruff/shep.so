import 'dart:io';
import 'package:code_assets/code_assets.dart';
import 'package:flutter_rust_bridge_hooks/flutter_rust_bridge_hooks.dart';

void main(List<String> args) async {
  await build(args, (input, output) async {
    if (!input.config.buildCodeAssets) return;
    output.dependencies.addAll([
      for (final path in [
        'rust/Cargo.toml',
        'rust/Cargo.lock',
        'rust/rust-toolchain.toml',
        '../shared/mail-core/Cargo.toml',
        '../shared/mail-content/Cargo.toml',
      ])
        input.packageRoot.resolve(path),
    ]);
    // Flutter's Linux Snap prepends a bundled Perl whose modules can resolve
    // from the host. OpenSSL needs a matched interpreter and module set.
    final snapPerl =
        Platform.isLinux &&
        (Platform.environment['PATH'] ?? '')
            .split(':')
            .any((p) => p.startsWith('/snap/flutter/')) &&
        File('/usr/bin/perl').existsSync();
    final environment = <String, String>{
      if (snapPerl) 'OPENSSL_SRC_PERL': '/usr/bin/perl',
    };
    final code = input.config.code;
    if (code.targetOS == OS.android) {
      // native_toolchain_rust 1.0.4 hardcodes API 35 compiler wrappers. Honor
      // Flutter's actual minSdk so C/OpenSSL cannot require newer Android APIs.
      final (rust, ndk) = switch (code.targetArchitecture) {
        Architecture.arm => (
          'armv7-linux-androideabi',
          'armv7a-linux-androideabi',
        ),
        Architecture.arm64 => (
          'aarch64-linux-android',
          'aarch64-linux-android',
        ),
        Architecture.x64 => ('x86_64-linux-android', 'x86_64-linux-android'),
        _ => throw UnsupportedError('Unsupported Android architecture'),
      };
      final directory = File.fromUri(code.cCompiler!.compiler).parent;
      final suffix = Platform.isWindows ? '.cmd' : '';
      final compiler =
          '${directory.path}/$ndk${code.android.targetNdkApi}-clang';
      final key = rust.replaceAll('-', '_');
      environment.addAll({
        'CC_$key': '$compiler$suffix',
        'CXX_$key': '$compiler++$suffix',
        'CARGO_TARGET_${key.toUpperCase()}_LINKER': '$compiler$suffix',
      });
    }
    await FlutterRustBridgeNativeAssetsBuilder(
      cratePath: 'rust',
      extraCargoEnvironmentVariables: environment,
    ).run(input: input, output: output);
  });
}
