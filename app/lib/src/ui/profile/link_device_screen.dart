import 'dart:async';

import 'package:flutter/material.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/design/typography.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/words.dart';

/// Linking a new device to this account: a code to type there, six digits
/// to compare, one approval here (ADR-0014).
class LinkDeviceScreen extends StatefulWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  @override
  State<LinkDeviceScreen> createState() => _LinkDeviceScreenState();
}

enum _Step { starting, waiting, asking, approving, linked, expired, failed }

class _LinkDeviceScreenState extends State<LinkDeviceScreen> {
  _Step _step = _Step.starting;
  String? _code;
  LinkApplicant? _applicant;
  String? _error;

  @override
  void initState() {
    super.initState();
    unawaited(_offer());
  }

  @override
  void dispose() {
    if (_step != _Step.linked) {
      widget.messenger.cancelLink();
    }
    super.dispose();
  }

  Future<void> _offer() async {
    setState(() {
      _step = _Step.starting;
      _applicant = null;
      _error = null;
    });
    try {
      final code = await widget.messenger.offerLink();
      if (!mounted) {
        return;
      }
      setState(() {
        _code = code;
        _step = _Step.waiting;
      });
      final applicant = await widget.messenger.nextApplicant();
      if (!mounted || _step != _Step.waiting) {
        return;
      }
      setState(() {
        _applicant = applicant;
        _step = applicant == null ? _Step.expired : _Step.asking;
      });
    } on MessengerFailure catch (failure) {
      _fail(failure);
    }
  }

  Future<void> _approve() async {
    setState(() => _step = _Step.approving);
    try {
      await widget.messenger.approveLink();
      if (mounted) {
        setState(() => _step = _Step.linked);
      }
    } on MessengerFailure catch (failure) {
      _fail(failure);
    }
  }

  void _fail(MessengerFailure failure) {
    if (mounted) {
      setState(() {
        _step = _Step.failed;
        _error = describeFailure(failure.code);
      });
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
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    'Add a device',
                    style: text.displayMedium?.copyWith(fontSize: 36),
                  ),
                  const SizedBox(height: ZSpace.xxl),
                  AnimatedSwitcher(
                    duration: ZMotion.base,
                    child: KeyedSubtree(
                      key: ValueKey(_step),
                      child: _body(context),
                    ),
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

  Widget _body(BuildContext context) {
    final text = context.text;
    return switch (_step) {
      _Step.starting => const Center(child: CircularProgressIndicator()),
      _Step.waiting => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'On your new device, tap “I already use Zelamar” and type this '
            'code.',
            style: text.bodyMedium,
          ),
          const SizedBox(height: ZSpace.lg),
          _Panel(
            child: SelectableText(
              _code ?? '',
              key: const Key('link.code'),
              textAlign: TextAlign.center,
              style: zelamarMono(size: 20)
                  .copyWith(color: context.colors.textPrimary),
            ),
          ),
          const SizedBox(height: ZSpace.xl),
          const _Busy('Waiting for your new device…'),
        ],
      ),
      _Step.asking || _Step.approving => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _Panel(
            child: Column(
              children: [
                Text(
                  'Link “${_applicant?.label ?? ''}”?',
                  key: const Key('link.applicant'),
                  style: text.titleMedium,
                  textAlign: TextAlign.center,
                ),
                const SizedBox(height: ZSpace.md),
                Text(
                  'It should show',
                  style: text.bodyMedium,
                  textAlign: TextAlign.center,
                ),
                const SizedBox(height: ZSpace.sm),
                Text(
                  _applicant?.matchCode ?? '',
                  key: const Key('link.match'),
                  style: zelamarMono(size: 32)
                      .copyWith(color: context.colors.textPrimary),
                ),
                const SizedBox(height: ZSpace.md),
                Text(
                  'It will see new messages in all your chats and write as '
                  'you. Earlier messages stay here.',
                  style: text.bodySmall,
                  textAlign: TextAlign.center,
                ),
              ],
            ),
          ),
          const SizedBox(height: ZSpace.xl),
          FilledButton(
            key: const Key('link.approve'),
            onPressed: _step == _Step.asking ? _approve : null,
            child: Text(_step == _Step.asking ? 'Add' : 'Adding…'),
          ),
          const SizedBox(height: ZSpace.sm),
          TextButton(
            key: const Key('link.cancel'),
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('The numbers differ: cancel'),
          ),
        ],
      ),
      _Step.linked => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            '“${_applicant?.label ?? 'Your device'}” is linked.',
            key: const Key('link.done'),
            style: text.titleMedium,
          ),
          const SizedBox(height: ZSpace.sm),
          Text('Your chats appear there in a moment.', style: text.bodyMedium),
          const SizedBox(height: ZSpace.xl),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('Done'),
          ),
        ],
      ),
      _Step.expired || _Step.failed => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            _error ?? 'The code expired.',
            key: const Key('link.error'),
            style: text.bodyMedium?.copyWith(color: context.colors.danger),
          ),
          const SizedBox(height: ZSpace.xl),
          FilledButton(
            key: const Key('link.again'),
            onPressed: _offer,
            child: const Text('Show a new code'),
          ),
        ],
      ),
    };
  }
}

class _Panel extends StatelessWidget {
  const new({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Container(
      padding: const EdgeInsets.all(ZSpace.xl),
      decoration: BoxDecoration(
        color: colors.surface,
        borderRadius: BorderRadius.circular(ZRadius.field),
        border: Border.all(color: colors.hairline),
      ),
      child: child,
    );
  }
}

class _Busy extends StatelessWidget {
  const new(this.label);

  final String label;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        SizedBox.square(
          dimension: 16,
          child: CircularProgressIndicator(
            strokeWidth: 2,
            color: context.colors.accent,
          ),
        ),
        const SizedBox(width: ZSpace.md),
        Flexible(child: Text(label, style: context.text.bodyMedium)),
      ],
    );
  }
}
