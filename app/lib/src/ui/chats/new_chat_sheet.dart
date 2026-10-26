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
  final _groupName = TextEditingController();
  bool _busy = false;
  String? _error;

  /// Starting a group rather than a chat with one person.
  bool _group = false;

  /// The people for a group, as they were added.
  final _people = <String>[];

  String? get _normalized => widget.messenger.normalizeUsername(_name.text);

  @override
  void dispose() {
    _name.dispose();
    _groupName.dispose();
    super.dispose();
  }

  /// Adds the typed name to the group's people.
  void _addPerson() {
    final username = _normalized;
    if (username == null) {
      return;
    }
    if (username == widget.messenger.username) {
      setState(() => _error = 'That’s you. Enter someone else’s name.');
      return;
    }
    setState(() {
      if (!_people.contains(username)) {
        _people.add(username);
      }
      _name.clear();
      _error = null;
    });
  }

  bool get _canCreate =>
      _groupName.text.trim().isNotEmpty && _people.isNotEmpty && !_busy;

  Future<void> _create() async {
    if (!_canCreate) {
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final chat = await widget.messenger.createGroup(
        _groupName.text.trim(),
        List.of(_people),
      );
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
          Text(_group ? 'New group' : 'New chat', style: text.headlineSmall),
          const SizedBox(height: ZSpace.xl),
          if (_group) ...[
            TextField(
              key: const Key('newGroup.name'),
              controller: _groupName,
              enabled: !_busy,
              autofocus: true,
              maxLength: 64,
              decoration: const InputDecoration(
                hintText: 'Group name',
                counterText: '',
              ),
              style: text.bodyLarge,
              onChanged: (_) => setState(() => _error = null),
            ),
            const SizedBox(height: ZSpace.md),
          ],
          TextField(
            key: const Key('newChat.username'),
            controller: _name,
            enabled: !_busy,
            autofocus: !_group,
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
              suffixIcon: _group
                  ? IconButton(
                      key: const Key('newGroup.add'),
                      tooltip: 'Add',
                      onPressed: _normalized != null && !_busy
                          ? _addPerson
                          : null,
                      icon: const Icon(Icons.add),
                    )
                  : null,
            ),
            style: text.bodyLarge,
            onChanged: (_) => setState(() => _error = null),
            onSubmitted: (_) => _group ? _addPerson() : _start(),
          ),
          if (_group && _people.isNotEmpty) ...[
            const SizedBox(height: ZSpace.md),
            Wrap(
              spacing: ZSpace.sm,
              runSpacing: ZSpace.sm,
              children: [
                for (final person in _people)
                  InputChip(
                    key: Key('newGroup.person.$person'),
                    label: Text('@$person'),
                    onDeleted: _busy
                        ? null
                        : () => setState(() => _people.remove(person)),
                  ),
              ],
            ),
          ],
          if (_error != null) ...[
            const SizedBox(height: ZSpace.md),
            Text(
              _error!,
              key: const Key('newChat.error'),
              style: text.bodyMedium?.copyWith(color: colors.danger),
            ),
          ],
          const SizedBox(height: ZSpace.xl),
          if (_group)
            FilledButton(
              key: const Key('newGroup.create'),
              onPressed: _canCreate ? _create : null,
              child: Text(_busy ? 'Creating…' : 'Create group'),
            )
          else
            FilledButton(
              key: const Key('newChat.start'),
              onPressed: _normalized != null && !_busy ? _start : null,
              child: Text(_busy ? 'Finding them…' : 'Start chat'),
            ),
          const SizedBox(height: ZSpace.sm),
          TextButton(
            key: const Key('newChat.mode'),
            onPressed: _busy
                ? null
                : () => setState(() {
                    _group = !_group;
                    _error = null;
                  }),
            child: Text(
              _group ? 'Chat with one person instead' : 'Start a group',
            ),
          ),
        ],
      ),
    );
  }
}
