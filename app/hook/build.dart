import 'package:flutter_rust_bridge_hooks/flutter_rust_bridge_hooks.dart';

/// Native Assets build hook: compiles the Rust bridge crate in `rust/` for
/// the target platform and bundles it with the app.
void main(List<String> args) async {
  await build(args, (input, output) async {
    await const FlutterRustBridgeNativeAssetsBuilder().run(
      input: input,
      output: output,
    );
  });
}
