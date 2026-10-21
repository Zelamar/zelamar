import 'dart:async';

import 'package:flutter/material.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/design/typography.dart';
import 'package:zelamar/src/lab/crypto_lab_screen.dart';
import 'package:zelamar/src/lab/lab_engine.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/avatar.dart';
import 'package:zelamar/src/ui/profile/devices_screen.dart';
import 'package:zelamar/src/ui/profile/recovery_key_screen.dart';
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
                              ? 'Kept on this device; only your recovery key '
                                    'can bring it to another. Your safety '
                                    'code with each person comes from it.'
                              : 'It stays on your account’s first device; '
                                    'this one is linked to it. Your safety '
                                    'code with each person comes from it.',
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
                  if (messenger.canLinkDevices)
                    ListTile(
                      key: const Key('profile.recovery'),
                      contentPadding: EdgeInsets.zero,
                      leading: Icon(Icons.key_outlined, color: colors.accent),
                      title: const Text('Recovery key'),
                      subtitle: Text(
                        messenger.recoveryKey == null
                            ? 'Not set up: losing this device loses your '
                                  'account'
                            : 'Brings your account back if you lose this '
                                  'device',
                      ),
                      trailing: const Icon(Icons.chevron_right),
                      onTap: () => Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          builder: (_) =>
                              RecoveryKeyScreen(messenger: messenger),
                        ),
                      ),
                    ),
                  SignInSwitch(messenger: messenger),
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

/// "Open at sign-in", where the platform has it (Windows): Zelamar starts in
/// the tray, so messages keep arriving after a restart.
class SignInSwitch extends StatefulWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  @override
  State<SignInSwitch> createState() => _SignInSwitchState();
}

class _SignInSwitchState extends State<SignInSwitch> {
  bool? _on;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    final on = await widget.messenger.startsAtSignIn();
    if (mounted) {
      setState(() => _on = on);
    }
  }

  Future<void> _set(bool on) async {
    final now = await widget.messenger.setStartsAtSignIn(on: on);
    if (mounted) {
      setState(() => _on = now);
    }
  }

  @override
  Widget build(BuildContext context) {
    final on = _on;
    if (on == null) {
      return const SizedBox.shrink();
    }
    return SwitchListTile(
      key: const Key('profile.signIn'),
      contentPadding: EdgeInsets.zero,
      secondary: Icon(Icons.login_rounded, color: context.colors.accent),
      title: const Text('Open at sign-in'),
      subtitle: const Text(
        'Start in the tray when you sign in to Windows, so messages keep '
        'arriving.',
      ),
      value: on,
      onChanged: (value) => unawaited(_set(value)),
    );
  }
}
