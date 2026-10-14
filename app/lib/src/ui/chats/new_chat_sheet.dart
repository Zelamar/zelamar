import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/words.dart';

/// Asks for someone's exact name and starts a chat with them. Returns the
/// chat id, or `null` if the sheet was dismissed.
Future<String?> showNewChatSheet(BuildContext context, Messenger messenger) {
  final colors = context.colors;
  return showModalBottomSheet<String>(
    context: context,
    isScrollControlled: true,
    backgroundColor: colors.surfaceRaised,
    shape: const RoundedRectangleBorder(
      borderRadius: BorderRadius.vertical(top: Radius.circular(ZRadius.card)),
    ),
    builder: (_) => NewChatSheet(messenger: messenger),
  );
}

class NewChatSheet extends StatefulWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  @override
  State<NewChatSheet> createState() => _NewChatSheetState();
}

class _NewChatSheetState extends State<NewChatSheet> {
  final _name = TextEditingController();
  bool _busy = false;
  String? _error;

  String? get _normalized => widget.messenger.normalizeUsername(_name.text);

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _start() async {
    final username = _normalized;
    if (username == null || _busy) {
      return;
    }
    if (username == widget.messenger.username) {
      setState(() => _error = 'That’s you. Enter someone else’s name.');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final chat = await widget.messenger.startChat(username);
      if (mounted) {
        Navigator.of(context).pop(chat);
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
    return Padding(
      padding: EdgeInsets.fromLTRB(
        ZSpace.gutter,
        ZSpace.md,
        ZSpace.gutter,
        ZSpace.xl + MediaQuery.viewInsetsOf(context).bottom,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Center(
            child: Container(
              width: 36,
              height: 4,
              decoration: BoxDecoration(
                color: colors.hairline,
                borderRadius: BorderRadius.circular(2),
              ),
            ),
          ),
          const SizedBox(height: ZSpace.xl),
          Text('New chat', style: text.headlineSmall),
          const SizedBox(height: ZSpace.sm),
          Text(
            'Enter their exact name. Zelamar has no public search, so no one '
            'can browse who’s here.',
            style: text.bodyMedium,
          ),
          const SizedBox(height: ZSpace.xl),
          TextField(
            key: const Key('newChat.username'),
            controller: _name,
            enabled: !_busy,
            autofocus: true,
            autocorrect: false,
            enableSuggestions: false,
            maxLength: 32,
            inputFormatters: [
              FilteringTextInputFormatter.allow(RegExp('[A-Za-z0-9_]')),
            ],
            decoration: InputDecoration(
              hintText: 'name',
              prefixText: '@',
              prefixStyle: text.bodyLarge?.copyWith(
                color: colors.textSecondary,
              ),
              counterText: '',
            ),
            style: text.bodyLarge,
            onChanged: (_) => setState(() => _error = null),
            onSubmitted: (_) => _start(),
          ),
          if (_error != null) ...[
            const SizedBox(height: ZSpace.md),
            Text(
              _error!,
              key: const Key('newChat.error'),
              style: text.bodyMedium?.copyWith(color: colors.danger),
            ),
          ],
          const SizedBox(height: ZSpace.xl),
          FilledButton(
            key: const Key('newChat.start'),
            onPressed: _normalized != null && !_busy ? _start : null,
            child: Text(_busy ? 'Finding them…' : 'Start chat'),
          ),
        ],
      ),
    );
  }
}
