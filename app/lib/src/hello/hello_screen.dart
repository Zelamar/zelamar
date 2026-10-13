import 'package:flutter/material.dart';
import 'package:zelamar/src/design/emblem.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/rust/api/diagnostics.dart';

/// Phase 0 smoke-test screen, dressed in the Ivory design system: proves
/// the Flutter UI talks to the Rust core.
///
/// Replaced by real onboarding in Phase 4.
class HelloScreen extends StatelessWidget {
  const new({required this.coreInfo, required this.greeting, super.key});

  final CoreInfo coreInfo;
  final String greeting;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;

    return Scaffold(
      body: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const SunsetZEmblem(),
            const SizedBox(height: ZSpace.xl),
            Text('Zelamar', style: text.displaySmall),
            const SizedBox(height: ZSpace.lg),
            Text(
              greeting,
              key: const Key('hello.greeting'),
              style: text.titleLarge,
            ),
            const SizedBox(height: ZSpace.sm),
            Text(
              'Core ${coreInfo.version} · Protocol ${coreInfo.protocolVersion}',
              key: const Key('hello.coreInfo'),
              style: text.bodySmall?.copyWith(color: colors.textTertiary),
            ),
          ],
        ),
      ),
    );
  }
}
