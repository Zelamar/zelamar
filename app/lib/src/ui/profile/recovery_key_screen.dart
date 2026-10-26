import 'package:flutter/material.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/design/typography.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/secret_clipboard.dart';
import 'package:zelamar/src/ui/words.dart';

/// The recovery key: what brings the account back on a new device after
/// this one is lost (ADR-0018). Only the device holding the account key
/// has one.
class RecoveryKeyScreen extends StatefulWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  @override
  State<RecoveryKeyScreen> createState() => _RecoveryKeyScreenState();
}

class _RecoveryKeyScreenState extends State<RecoveryKeyScreen> {
  bool _busy = false;
  bool _shown = false;
  bool _copied = false;
  String? _error;

  Messenger get _messenger => widget.messenger;

  Future<void> _create() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await _messenger.newRecoveryKey();
      if (mounted) {
        setState(() {
          _shown = true;
          _copied = false;
        });
      }
    } on MessengerFailure catch (failure) {
      if (mounted) {
        setState(() => _error = describeFailure(failure.code));
      }
    } finally {
      if (mounted) {
        setState(() => _busy = false);
      }
    }
  }

  Future<void> _replace() async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Replace your recovery key?'),
        content: const Text(
          'Your current key stops working. Write down the new one.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          TextButton(
            key: const Key('recovery.confirmReplace'),
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Replace'),
          ),
        ],
      ),
    );
    if (sure ?? false) {
      await _create();
    }
  }

  Future<void> _copy(String key) async {
    await copySecret(key);
    if (mounted) {
      setState(() => _copied = true);
    }
  }

  @override
  Widget build(BuildContext context) {
    final text = context.text;
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
              child: ListenableBuilder(
                listenable: _messenger,
                builder: (context, _) {
                  final key = _messenger.recoveryKey;
                  return Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Text(
                        'Recovery key',
                        style: text.displayMedium?.copyWith(fontSize: 36),
                      ),
                      const SizedBox(height: ZSpace.sm),
                      Text(
                        'Brings your account back if you lose this device.',
                        style: text.bodyMedium,
                      ),
                      const SizedBox(height: ZSpace.xl),
                      if (key == null)
                        ..._none(context)
                      else
                        ..._key(context, key),
                      if (_error != null)
                        Padding(
                          padding: const EdgeInsets.only(top: ZSpace.lg),
                          child: Text(
                            _error!,
                            key: const Key('recovery.error'),
                            style: text.bodyMedium?.copyWith(
                              color: context.colors.danger,
                            ),
                          ),
                        ),
                      const SizedBox(height: ZSpace.xl),
                    ],
                  );
                },
              ),
            ),
          ),
        ),
      ),
    );
  }

  List<Widget> _none(BuildContext context) => [
    FilledButton(
      key: const Key('recovery.create'),
      onPressed: _busy ? null : _create,
      child: Text(_busy ? 'Creating…' : 'Create recovery key'),
    ),
  ];

  List<Widget> _key(BuildContext context, String key) {
    final colors = context.colors;
    final text = context.text;
    final saved = _messenger.recoveryKeySaved;
    return [
      Container(
        padding: const EdgeInsets.all(ZSpace.xl),
        decoration: BoxDecoration(
          color: colors.surface,
          borderRadius: BorderRadius.circular(ZRadius.field),
          border: Border.all(color: colors.hairline),
        ),
        child: Column(
          children: [
            // One line on any screen: the key reads as one thing.
            FittedBox(
              key: const Key('recovery.key'),
              fit: BoxFit.scaleDown,
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  for (final (index, group) in key.split('-').indexed)
                    Padding(
                      padding: EdgeInsets.only(
                        left: index == 0 ? 0 : ZSpace.md,
                      ),
                      child: Text(
                        _shown ? group : '•' * group.length,
                        style: zelamarMono(size: 20)
                            .copyWith(color: colors.textPrimary),
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(height: ZSpace.lg),
            Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                TextButton.icon(
                  key: const Key('recovery.show'),
                  onPressed: () => setState(() => _shown = !_shown),
                  icon: Icon(
                    _shown
                        ? Icons.visibility_off_outlined
                        : Icons.visibility_outlined,
                    size: 18,
                  ),
                  label: Text(_shown ? 'Hide' : 'Show'),
                ),
                const SizedBox(width: ZSpace.sm),
                TextButton.icon(
                  key: const Key('recovery.copy'),
                  onPressed: () => _copy(key),
                  icon: Icon(
                    _copied ? Icons.check_rounded : Icons.copy_rounded,
                    size: 18,
                  ),
                  label: Text(_copied ? 'Copied' : 'Copy'),
                ),
              ],
            ),
          ],
        ),
      ),
      const SizedBox(height: ZSpace.md),
      Row(
        children: [
          Icon(
            saved ? Icons.lock_outline_rounded : Icons.cloud_off_outlined,
            size: 16,
            color: saved ? colors.online : colors.textTertiary,
          ),
          const SizedBox(width: ZSpace.sm),
          Expanded(
            child: Text(
              saved ? 'Saved' : 'Will save when you’re online',
              key: const Key('recovery.status'),
              style: text.bodySmall,
            ),
          ),
        ],
      ),
      const SizedBox(height: ZSpace.xl),
      Text(
        'Write it down and keep it safe. Anyone who has it can get into your '
        'account.',
        key: const Key('recovery.warning'),
        style: text.bodyMedium?.copyWith(color: colors.textPrimary),
      ),
      const SizedBox(height: ZSpace.lg),
      TextButton(
        key: const Key('recovery.replace'),
        onPressed: _busy ? null : _replace,
        child: Text(_busy ? 'Creating…' : 'Replace key'),
      ),
    ];
  }
}
