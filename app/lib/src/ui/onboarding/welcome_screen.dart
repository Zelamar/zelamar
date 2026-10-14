import 'package:flutter/material.dart';
import 'package:zelamar/src/design/emblem.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/lab/crypto_lab_screen.dart';
import 'package:zelamar/src/lab/lab_engine.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/onboarding/account_screen.dart';

/// The first thing anyone sees: the emblem, the name, one promise, and one
/// way forward.
class WelcomeScreen extends StatelessWidget {
  const new({required this.messenger, required this.startLab, super.key});

  final Messenger messenger;
  final LabEngineFactory startLab;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;

    return Scaffold(
      body: SafeArea(
        child: Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: ZSpace.gutter),
              child: Column(
                children: [
                  const Spacer(flex: 3),
                  const SunsetZEmblem(size: 128),
                  const SizedBox(height: ZSpace.xxl),
                  Text('Zelamar', style: text.displaySmall),
                  const SizedBox(height: ZSpace.md),
                  Text(
                    'Private by design.',
                    style: text.bodyLarge?.copyWith(
                      color: colors.textSecondary,
                    ),
                    textAlign: TextAlign.center,
                  ),
                  const Spacer(flex: 4),
                  SizedBox(
                    width: double.infinity,
                    child: FilledButton(
                      key: const Key('welcome.create'),
                      onPressed: () => Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          builder: (_) => AccountScreen(messenger: messenger),
                        ),
                      ),
                      child: const Text('Create your account'),
                    ),
                  ),
                  const SizedBox(height: ZSpace.md),
                  Text(
                    'No phone number. No email. Just a name.',
                    style: text.bodySmall?.copyWith(color: colors.textTertiary),
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: ZSpace.lg),
                  TextButton(
                    key: const Key('welcome.lab'),
                    onPressed: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => CryptoLabScreen(startEngine: startLab),
                      ),
                    ),
                    child: const Text('See how your messages are protected'),
                  ),
                  const SizedBox(height: ZSpace.md),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
