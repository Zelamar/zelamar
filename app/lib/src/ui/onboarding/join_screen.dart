import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/design/typography.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/onboarding/recover_screen.dart';
import 'package:zelamar/src/ui/words.dart';

/// Joining an account this person already has, by the code their other
/// device shows (ADR-0014).
class JoinScreen extends StatefulWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  @override
  State<JoinScreen> createState() => _JoinScreenState();
}

class _JoinScreenState extends State<JoinScreen> {
  final _code = TextEditingController();
  bool _busy = false;
  String? _match;
  String? _error;

  /// Letters and digits typed so far; dashes and spaces do not count.
  int get _typed => _code.text.replaceAll(RegExp(r'[\s-]'), '').length;

  bool get _ready => _typed == 20 && !_busy;

  @override
  void dispose() {
    if (_busy) {
      widget.messenger.cancelJoin();
    }
    _code.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await widget.messenger.joinAccount(
        code: _code.text,
        onMatch: (match) {
          if (mounted) {
            setState(() => _match = match);
          }
        },
      );
      if (mounted) {
        // The root now shows the chats; leave onboarding behind.
        Navigator.of(context).popUntil((route) => route.isFirst);
      }
    } on MessengerFailure catch (failure) {
      if (mounted) {
        setState(() {
          _busy = false;
          _match = null;
          _error = failure.code == 'cancelled'
              ? null
              : describeFailure(failure.code);
        });
      }
    }
  }

  void _cancel() {
    widget.messenger.cancelJoin();
    setState(() {
      _busy = false;
      _match = null;
    });
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
              constraints: const BoxConstraints(maxWidth: 420),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    'Link this device',
                    style: text.displayMedium?.copyWith(fontSize: 36),
                  ),
                  const SizedBox(height: ZSpace.sm),
                  Text(
                    'On your other device: You → Devices → Link a device.',
                    style: text.bodyMedium,
                  ),
                  const SizedBox(height: ZSpace.xxl),
                  AnimatedSwitcher(
                    duration: ZMotion.base,
                    child: _match == null
                        ? _form(context)
                        : _Waiting(match: _match!, onCancel: _cancel),
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

  Widget _form(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    return Column(
      key: const Key('join.form'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TextField(
          key: const Key('join.code'),
          controller: _code,
          enabled: !_busy,
          autofocus: true,
          autocorrect: false,
          enableSuggestions: false,
          maxLength: 29,
          inputFormatters: [
            FilteringTextInputFormatter.allow(RegExp(r'[A-Za-z0-9\s-]')),
          ],
          decoration: const InputDecoration(
            hintText: 'xxxxx-xxxxx-xxxxx-xxxxx',
            counterText: '',
          ),
          style: zelamarMono(size: 16).copyWith(color: colors.textPrimary),
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
                    key: const Key('join.error'),
                    style: text.bodyMedium?.copyWith(color: colors.danger),
                  ),
                ),
        ),
        FilledButton(
          key: const Key('join.continue'),
          onPressed: _ready ? _submit : null,
          child: Text(_busy ? 'Sending…' : 'Continue'),
        ),
        const SizedBox(height: ZSpace.lg),
        Text(
          'Earlier messages stay on your other device.',
          style: text.bodySmall?.copyWith(color: colors.textTertiary),
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: ZSpace.xl),
        TextButton(
          key: const Key('join.recover'),
          onPressed: _busy
              ? null
              : () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => RecoverScreen(messenger: widget.messenger),
                  ),
                ),
          child: const Text('Lost your other device? Use your recovery key'),
        ),
      ],
    );
  }
}

/// The request is in: the person compares six digits and approves on the
/// other device.
class _Waiting extends StatelessWidget {
  const new({required this.match, required this.onCancel});

  final String match;
  final VoidCallback onCancel;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    return Column(
      key: const Key('join.waiting'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          padding: const EdgeInsets.all(ZSpace.xl),
          decoration: BoxDecoration(
            color: colors.surface,
            borderRadius: BorderRadius.circular(ZRadius.field),
            border: Border.all(color: colors.hairline),
          ),
          child: Column(
            children: [
              Text(
                'Your other device should show',
                style: text.bodyMedium,
                textAlign: TextAlign.center,
              ),
              const SizedBox(height: ZSpace.md),
              Text(
                match,
                key: const Key('join.match'),
                style: zelamarMono(size: 32)
                    .copyWith(color: colors.textPrimary),
              ),
              const SizedBox(height: ZSpace.md),
              Text(
                'If it does, approve there. If not, cancel.',
                style: text.bodySmall,
                textAlign: TextAlign.center,
              ),
            ],
          ),
        ),
        const SizedBox(height: ZSpace.xl),
        Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            SizedBox.square(
              dimension: 16,
              child: CircularProgressIndicator(
                strokeWidth: 2,
                color: colors.accent,
              ),
            ),
            const SizedBox(width: ZSpace.md),
            Text('Waiting for approval…', style: text.bodyMedium),
          ],
        ),
        const SizedBox(height: ZSpace.lg),
        TextButton(
          key: const Key('join.cancel'),
          onPressed: onCancel,
          child: const Text('Cancel'),
        ),
      ],
    );
  }
}
