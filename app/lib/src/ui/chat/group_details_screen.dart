import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/avatar.dart';
import 'package:zelamar/src/ui/words.dart';

/// A group's name and people: rename it, add someone, or leave
/// (ADR-0015).
class GroupDetailsScreen extends StatefulWidget {
  const new({required this.messenger, required this.chat, super.key});

  final Messenger messenger;
  final String chat;

  @override
  State<GroupDetailsScreen> createState() => _GroupDetailsScreenState();
}

class _GroupDetailsScreenState extends State<GroupDetailsScreen> {
  List<MemberView>? _members;
  bool _busy = false;

  Messenger get _messenger => widget.messenger;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    try {
      final members = await _messenger.members(widget.chat);
      if (mounted) {
        setState(() => _members = members);
      }
    } on MessengerFailure catch (failure) {
      _say(describeFailure(failure.code));
    }
  }

  void _say(String message) {
    if (mounted) {
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text(message)));
    }
  }

  Future<void> _run(Future<void> Function() action) async {
    setState(() => _busy = true);
    try {
      await action();
    } on MessengerFailure catch (failure) {
      _say(describeFailure(failure.code));
    } finally {
      if (mounted) {
        setState(() => _busy = false);
      }
    }
  }

  Future<void> _rename(String current) async {
    final name = await _ask(
      title: 'Rename the group',
      hint: 'Group name',
      initial: current,
      action: 'Rename',
      maxLength: 64,
    );
    if (name != null && name != current) {
      await _run(() => _messenger.renameChat(widget.chat, name));
    }
  }

  Future<void> _add() async {
    final typed = await _ask(
      title: 'Add someone',
      hint: 'name',
      prefix: '@',
      action: 'Add',
      maxLength: 32,
      allowed: RegExp('[A-Za-z0-9_]'),
    );
    final username = typed == null ? null : _messenger.normalizeUsername(typed);
    if (typed != null && username == null) {
      _say(describeFailure('invalid_username'));
      return;
    }
    if (username != null) {
      await _run(() async {
        await _messenger.addMembers(widget.chat, [username]);
        await _load();
      });
    }
  }

  Future<void> _leave() async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Leave this group?'),
        content: const Text(
          'It disappears from this device with all its messages, and the '
          'others see that you left.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          TextButton(
            key: const Key('group.leave.confirm'),
            onPressed: () => Navigator.of(context).pop(true),
            style: TextButton.styleFrom(foregroundColor: context.colors.danger),
            child: const Text('Leave'),
          ),
        ],
      ),
    );
    if (sure == true) {
      await _run(() async {
        await _messenger.leaveChat(widget.chat);
        if (mounted) {
          Navigator.of(context).popUntil((route) => route.isFirst);
        }
      });
    }
  }

  /// One line of text from the user, or `null` if they cancelled.
  Future<String?> _ask({
    required String title,
    required String hint,
    required String action,
    required int maxLength,
    String initial = '',
    String? prefix,
    RegExp? allowed,
  }) async {
    final value = await showDialog<String>(
      context: context,
      builder: (_) => _LineDialog(
        title: title,
        hint: hint,
        action: action,
        maxLength: maxLength,
        initial: initial,
        prefix: prefix,
        allowed: allowed,
      ),
    );
    return value == null || value.isEmpty ? null : value;
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    return ListenableBuilder(
      listenable: _messenger,
      builder: (context, _) {
        final preview = _messenger.chats
            .where((chat) => chat.id == widget.chat)
            .firstOrNull;
        final name = preview?.title ?? '';
        final members = _members;
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
                      Center(child: Avatar(name, size: 88)),
                      const SizedBox(height: ZSpace.lg),
                      Row(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Flexible(
                            child: Text(
                              name,
                              key: const Key('group.name'),
                              style: text.headlineSmall,
                              textAlign: TextAlign.center,
                            ),
                          ),
                          IconButton(
                            key: const Key('group.rename'),
                            tooltip: 'Rename',
                            onPressed: _busy ? null : () => _rename(name),
                            icon: Icon(
                              Icons.edit_outlined,
                              color: colors.textSecondary,
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: ZSpace.xl),
                      Text(
                        members == null
                            ? 'PEOPLE'
                            : 'PEOPLE · ${members.length + 1}',
                        style: text.labelMedium?.copyWith(
                          color: colors.textTertiary,
                        ),
                      ),
                      const SizedBox(height: ZSpace.sm),
                      const ListTile(
                        contentPadding: EdgeInsets.zero,
                        title: Text('You'),
                        leading: SizedBox.square(dimension: 36),
                      ),
                      if (members == null)
                        const Center(child: CircularProgressIndicator())
                      else
                        for (final member in members)
                          ListTile(
                            contentPadding: EdgeInsets.zero,
                            leading: Avatar(member.title, size: 36),
                            title: Text(member.title),
                          ),
                      const SizedBox(height: ZSpace.lg),
                      OutlinedButton.icon(
                        key: const Key('group.add'),
                        onPressed: _busy ? null : _add,
                        icon: const Icon(Icons.person_add_alt_1_outlined),
                        label: const Text('Add someone'),
                      ),
                      const SizedBox(height: ZSpace.md),
                      TextButton(
                        key: const Key('group.leave'),
                        onPressed: _busy ? null : _leave,
                        style: TextButton.styleFrom(
                          foregroundColor: colors.danger,
                        ),
                        child: const Text('Leave group'),
                      ),
                      const SizedBox(height: ZSpace.xl),
                    ],
                  ),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

/// Asks for one line of text.
class _LineDialog extends StatefulWidget {
  const new({
    required this.title,
    required this.hint,
    required this.action,
    required this.maxLength,
    required this.initial,
    this.prefix,
    this.allowed,
  });

  final String title;
  final String hint;
  final String action;
  final int maxLength;
  final String initial;
  final String? prefix;
  final RegExp? allowed;

  @override
  State<_LineDialog> createState() => _LineDialogState();
}

class _LineDialogState extends State<_LineDialog> {
  late final _controller = TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _done() => Navigator.of(context).pop(_controller.text.trim());

  @override
  Widget build(BuildContext context) {
    final allowed = widget.allowed;
    return AlertDialog(
      title: Text(widget.title),
      content: TextField(
        key: const Key('group.field'),
        controller: _controller,
        autofocus: true,
        autocorrect: false,
        maxLength: widget.maxLength,
        inputFormatters: [
          if (allowed != null) FilteringTextInputFormatter.allow(allowed),
        ],
        decoration: InputDecoration(
          hintText: widget.hint,
          prefixText: widget.prefix,
          counterText: '',
        ),
        onSubmitted: (_) => _done(),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(
          key: const Key('group.field.done'),
          onPressed: _done,
          child: Text(widget.action),
        ),
      ],
    );
  }
}
