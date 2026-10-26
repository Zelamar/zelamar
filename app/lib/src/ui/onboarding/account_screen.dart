import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/words.dart';

/// Choosing a name: the only thing Zelamar asks of anyone.
class AccountScreen extends StatefulWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  @override
  State<AccountScreen> createState() => _AccountScreenState();
}

class _AccountScreenState extends State<AccountScreen> {
  final _name = TextEditingController();
  final _invite = TextEditingController();
  bool _busy = false;
  String? _error;

  String? get _normalized => widget.messenger.normalizeUsername(_name.text);

  bool get _ready =>
      _normalized != null && _invite.text.trim().isNotEmpty && !_busy;

  @override
  void dispose() {
    _name.dispose();
    _invite.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final username = _normalized;
    if (username == null) {
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await widget.messenger.createAccount(
        username: username,
        invite: _invite.text.trim(),
      );
      if (mounted) {
        // The root now shows the chats; leave onboarding behind.
        Navigator.of(context).popUntil((route) => route.isFirst);
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
    final typed = _name.text.isNotEmpty;
    final valid = _normalized != null;

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
                    'Choose your name',
                    style: text.displayMedium?.copyWith(fontSize: 36),
                  ),
                  const SizedBox(height: ZSpace.xxl),
                  TextField(
                    key: const Key('account.username'),
                    controller: _name,
                    enabled: !_busy,
                    autofocus: true,
                    autocorrect: false,
                    enableSuggestions: false,
                    textInputAction: TextInputAction.next,
                    maxLength: 32,
                    inputFormatters: [
                      FilteringTextInputFormatter.allow(RegExp('[A-Za-z0-9_]')),
                    ],
                    decoration: InputDecoration(
                      hintText: 'yourname',
                      prefixText: '@',
                      prefixStyle: text.bodyLarge?.copyWith(
                        color: colors.textSecondary,
                      ),
                      counterText: '',
                    ),
                    style: text.bodyLarge,
                    onChanged: (_) => setState(() => _error = null),
                  ),
                  // Said only when the name breaks a rule.
                  if (typed && !valid)
                    Padding(
                      padding: const EdgeInsets.fromLTRB(
                        ZSpace.lg,
                        ZSpace.sm,
                        ZSpace.lg,
                        0,
                      ),
                      child: Text(
                        describeFailure('invalid_username'),
                        key: const Key('account.rule'),
                        style: text.bodySmall?.copyWith(color: colors.danger),
                      ),
                    ),
                  const SizedBox(height: ZSpace.md),
                  TextField(
                    key: const Key('account.invite'),
                    controller: _invite,
                    enabled: !_busy,
                    autocorrect: false,
                    enableSuggestions: false,
                    decoration: const InputDecoration(hintText: 'Invite code'),
                    style: text.bodyLarge,
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
                              key: const Key('account.error'),
                              style: text.bodyMedium?.copyWith(
                                color: colors.danger,
                              ),
                            ),
                          ),
                  ),
                  FilledButton(
                    key: const Key('account.continue'),
                    onPressed: _ready ? _submit : null,
                    child: _busy
                        ? Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              SizedBox.square(
                                dimension: 16,
                                child: CircularProgressIndicator(
                                  strokeWidth: 2,
                                  color: colors.onAccent,
                                ),
                              ),
                              const SizedBox(width: ZSpace.md),
                              const Flexible(
                                child: Text(
                                  'Creating your keys…',
                                  overflow: TextOverflow.ellipsis,
                                ),
                              ),
                            ],
                          )
                        : const Text('Continue'),
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
