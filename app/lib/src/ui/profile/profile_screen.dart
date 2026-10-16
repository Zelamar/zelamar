import 'package:flutter/material.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/design/typography.dart';
import 'package:zelamar/src/lab/crypto_lab_screen.dart';
import 'package:zelamar/src/lab/lab_engine.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/avatar.dart';
import 'package:zelamar/src/ui/profile/devices_screen.dart';
import 'package:zelamar/src/version.dart';

/// "You": the account's name and key, and how Zelamar protects it.
class ProfileScreen extends StatelessWidget {
  const new({required this.messenger, required this.startLab, super.key});

  final Messenger messenger;
  final LabEngineFactory startLab;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    final name = '@${messenger.username ?? ''}';
    return Scaffold(
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        surfaceTintColor: Colors.transparent,
      ),
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.symmetric(horizontal: ZSpace.gutter),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 440),
              child: Column(
                children: [
                  Avatar(name, size: 88),
                  const SizedBox(height: ZSpace.lg),
                  Text(
                    name,
                    key: const Key('profile.name'),
                    style: text.headlineSmall,
                  ),
                  const SizedBox(height: ZSpace.xxl),
                  Container(
                    width: double.infinity,
                    padding: const EdgeInsets.all(ZSpace.lg),
                    decoration: BoxDecoration(
                      color: colors.surface,
                      borderRadius: BorderRadius.circular(ZRadius.field),
                      border: Border.all(color: colors.hairline),
                    ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          'YOUR KEY',
                          style: text.labelMedium?.copyWith(
                            color: colors.textTertiary,
                          ),
                        ),
                        const SizedBox(height: ZSpace.sm),
                        SelectableText(
                          messenger.fingerprint ?? '',
                          key: const Key('profile.fingerprint'),
                          style: zelamarMono(size: 14)
                              .copyWith(color: colors.textPrimary),
                        ),
                        const SizedBox(height: ZSpace.sm),
                        Text(
                          messenger.canLinkDevices
                              ? 'Created on this device; it never leaves it. '
                                    'Your safety code with each person comes '
                                    'from it.'
                              : 'It stays on the device that created your '
                                    'account; this one is linked to it. Your '
                                    'safety code with each person comes from '
                                    'it.',
                          key: const Key('profile.keyNote'),
                          style: text.bodySmall,
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(height: ZSpace.lg),
                  ListTile(
                    key: const Key('profile.devices'),
                    contentPadding: EdgeInsets.zero,
                    leading: Icon(Icons.devices_outlined, color: colors.accent),
                    title: const Text('Devices'),
                    subtitle: Text(
                      messenger.canLinkDevices
                          ? 'Use Zelamar on a phone and a computer'
                          : 'Linked to your account',
                    ),
                    trailing: const Icon(Icons.chevron_right),
                    onTap: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => DevicesScreen(messenger: messenger),
                      ),
                    ),
                  ),
                  ListTile(
                    key: const Key('profile.lab'),
                    contentPadding: EdgeInsets.zero,
                    leading: Icon(Icons.shield_outlined, color: colors.accent),
                    title: const Text('See how your messages are protected'),
                    trailing: const Icon(Icons.chevron_right),
                    onTap: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => CryptoLabScreen(startEngine: startLab),
                      ),
                    ),
                  ),
                  const SizedBox(height: ZSpace.xxl),
                  Text(
                    'Zelamar $appVersion',
                    style: text.bodySmall?.copyWith(color: colors.textTertiary),
                  ),
                  const SizedBox(height: ZSpace.xl),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
