import 'package:flutter/material.dart';
import 'package:zelamar/src/hello/hello_screen.dart';
import 'package:zelamar/src/rust/api/diagnostics.dart';

/// Root widget of Zelamar.
///
/// Receives everything it needs from the Rust core as plain values, so the
/// widget tree stays testable without loading the native library.
class ZelamarApp extends StatelessWidget {
  const new({required this.coreInfo, required this.greeting, super.key});

  final CoreInfo coreInfo;
  final String greeting;

  // Placeholder palette until the visual direction is chosen in Phase 1.
  static const Color _seed = Color(0xFF0B4F5C);

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Zelamar',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(colorSchemeSeed: _seed),
      darkTheme: ThemeData(colorSchemeSeed: _seed, brightness: Brightness.dark),
      home: HelloScreen(coreInfo: coreInfo, greeting: greeting),
    );
  }
}
