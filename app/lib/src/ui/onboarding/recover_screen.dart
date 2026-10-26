import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/design/typography.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/profile/devices_screen.dart';
import 'package:zelamar/src/ui/words.dart';

/// Recovering an account on a new device with its recovery key, after the
/// device holding the account was lost (ADR-0018).
class RecoverScreen extends StatefulWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  @override
  State<RecoverScreen> createState() => _RecoverScreenState();
}

class _RecoverScreenState extends State<RecoverScreen> {
  final _name = TextEditingController();
  final _key = TextEditingController();
  bool _busy = false;
  String? _error;

  /// Letters and digits of the key typed so far.
  int get _typed => _key.text.replaceAll(RegExp(r'[\s-]'), '').length;

  bool get _ready =>
      !_busy &&
      _typed == 25 &&
      widget.messenger.normalizeUsername(_username) != null;

  String get _username => _name.text.trim().replaceFirst(RegExp('^@'), '');

  @override
  void dispose() {
    _name.dispose();
    _key.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await widget.messenger.recoverAccount(
        username: _username,
        key: _key.text,
      );
      if (mounted) {
        // The root now shows the chats; the lost device goes next.
        Navigator.of(context)
          ..popUntil((route) => route.isFirst)
          ..push(
            MaterialPageRoute<void>(
              builder: (_) =>
                  DevicesScreen(messenger: widget.messenger, recovered: true),
            ),
          );
      }
    } on MessengerFailure catch (failure) {
      if (mounted) {
        setState(() {
          _busy = false;
          _error = describeFailure(failure.code);
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
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
              constraints: const BoxConstraints(maxWidth: 420),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    'Recover your account',
                    style: text.displayMedium?.copyWith(fontSize: 36),
                  ),
                  const SizedBox(height: ZSpace.sm),
                  Text(
                    'Type your name and the recovery key you wrote down.',
                    style: text.bodyMedium,
                  ),
                  const SizedBox(height: ZSpace.xxl),
                  TextField(
                    key: const Key('recover.name'),
                    controller: _name,
                    enabled: !_busy,
                    autofocus: true,
                    autocorrect: false,
                    enableSuggestions: false,
                    textInputAction: TextInputAction.next,
                    decoration: const InputDecoration(
                      hintText: 'yourname',
                      prefixText: '@',
                    ),
                    onChanged: (_) => setState(() => _error = null),
                  ),
                  const SizedBox(height: ZSpace.md),
                  TextField(
                    key: const Key('recover.key'),
                    controller: _key,
                    enabled: !_busy,
                    autocorrect: false,
                    enableSuggestions: false,
                    maxLength: 36,
                    inputFormatters: [
                      FilteringTextInputFormatter.allow(
                        RegExp(r'[A-Za-z0-9\s-]'),
                      ),
                    ],
                    decoration: const InputDecoration(
                      hintText: 'xxxxx-xxxxx-xxxxx-xxxxx-xxxxx',
                      counterText: '',
                    ),
                    style: zelamarMono(size: 16)
                        .copyWith(color: colors.textPrimary),
                    onChanged: (_) => setState(() => _error = null),
                    onSubmitted: (_) => _ready ? _submit() : null,
                  ),
                  const SizedBox(height: ZSpace.xl),
                  AnimatedSwitcher(
                    duration: ZMotion.base,
                    child: _error == null
                        ? const SizedBox.shrink()
                        : Padding(
                            padding: const EdgeInsets.only(bottom: ZSpace.lg),
                            child: Text(
                              _error!,
                              key: const Key('recover.error'),
                              style: text.bodyMedium?.copyWith(
                                color: colors.danger,
                              ),
                            ),
                          ),
                  ),
                  FilledButton(
                    key: const Key('recover.continue'),
                    onPressed: _ready ? _submit : null,
                    child: Text(_busy ? 'Recovering…' : 'Recover'),
                  ),
                  const SizedBox(height: ZSpace.lg),
                  Text(
                    'Old messages stay on the lost device. Your chats come '
                    'back here.',
                    style: text.bodySmall?.copyWith(color: colors.textTertiary),
                    textAlign: TextAlign.center,
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
