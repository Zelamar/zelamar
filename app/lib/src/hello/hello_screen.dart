import 'package:flutter/material.dart';
import 'package:zelamar/src/rust/api/diagnostics.dart';

/// Phase 0 smoke-test screen: proves the Flutter UI talks to the Rust core.
///
/// Replaced by real onboarding in Phase 4.
class HelloScreen extends StatelessWidget {
  const new({required this.coreInfo, required this.greeting, super.key});

  final CoreInfo coreInfo;
  final String greeting;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final textTheme = theme.textTheme;
    final muted = theme.colorScheme.onSurfaceVariant;

    return Scaffold(
      body: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              'Zelamar',
              style: textTheme.displayMedium?.copyWith(
                fontWeight: FontWeight.w300,
                letterSpacing: 2,
              ),
            ),
            const SizedBox(height: 24),
            Text(
              greeting,
              key: const Key('hello.greeting'),
              style: textTheme.titleLarge,
            ),
            const SizedBox(height: 8),
            Text(
              'Core ${coreInfo.version} · Protocol ${coreInfo.protocolVersion}',
              key: const Key('hello.coreInfo'),
              style: textTheme.bodySmall?.copyWith(color: muted),
            ),
          ],
        ),
      ),
    );
  }
}
