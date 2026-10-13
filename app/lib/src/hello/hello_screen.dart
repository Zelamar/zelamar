import 'package:flutter/material.dart';
import 'package:zelamar/src/design/emblem.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/lab/crypto_lab_screen.dart';
import 'package:zelamar/src/lab/lab_engine.dart';
import 'package:zelamar/src/rust/api/diagnostics.dart';

/// Phase 0 smoke-test screen, dressed in the Ivory design system: proves
/// the Flutter UI talks to the Rust core, and opens the Crypto Lab.
///
/// Replaced by real onboarding in Phase 4.
class HelloScreen extends StatelessWidget {
  const new({
    required this.coreInfo,
    required this.greeting,
    required this.startLab,
    super.key,
  });

  final CoreInfo coreInfo;
  final String greeting;
  final LabEngineFactory startLab;

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
            const SizedBox(height: ZSpace.xxl),
            FilledButton.icon(
              key: const Key('hello.openLab'),
              onPressed: () => Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => CryptoLabScreen(startEngine: startLab),
                ),
              ),
              icon: const Icon(Icons.science_outlined, size: 18),
              label: const Text('Open Crypto Lab'),
            ),
          ],
        ),
      ),
    );
  }
}
