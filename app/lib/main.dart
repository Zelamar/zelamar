import 'package:flutter/widgets.dart';
import 'package:zelamar/src/app.dart';
import 'package:zelamar/src/rust/api/diagnostics.dart' as core;
import 'package:zelamar/src/rust/frb_generated.dart';

Future<void> main() async {
  await RustLib.init();
  runApp(
    ZelamarApp(
      coreInfo: core.coreInfo(),
      greeting: core.greet(name: ''),
    ),
  );
}
