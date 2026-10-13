import 'package:flutter/material.dart';
import 'package:zelamar/src/design/theme.dart';
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

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Zelamar',
      debugShowCheckedModeBanner: false,
      theme: zelamarTheme(Brightness.light),
      darkTheme: zelamarTheme(Brightness.dark),
      home: HelloScreen(coreInfo: coreInfo, greeting: greeting),
    );
  }
}
