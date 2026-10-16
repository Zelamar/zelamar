import 'dart:async';
import 'dart:math';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/outgoing_file.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/avatar.dart';
import 'package:zelamar/src/ui/chat/attachments.dart';
import 'package:zelamar/src/ui/chat/group_details_screen.dart';
import 'package:zelamar/src/ui/chat/safety_code.dart';
import 'package:zelamar/src/ui/words.dart';

/// One conversation.
class ChatScreen extends StatefulWidget {
  const new({required this.messenger, required this.chat, super.key});

  final Messenger messenger;

  /// The chat id.
  final String chat;

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> with WidgetsBindingObserver {
  static const _page = 60;

  List<MessageView> _messages = const [];
  final _input = TextEditingController();

  /// The message the next one answers, if any.
  MessageView? _replying;

  /// The user's own message being edited, if any.
  MessageView? _editing;
  bool _loadingOlder = false;
  bool _complete = false;
  StreamSubscription<String?>? _changes;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _changes = widget.messenger.chatChanged.listen((chat) {
      if (chat == null || chat == widget.chat) {
        unawaited(_reload());
      }
    });
    unawaited(_reload());
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    unawaited(_changes?.cancel());
    _input.dispose();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      unawaited(_markRead());
    }
  }

  /// Marks the chat read while the user can actually see it.
  Future<void> _markRead() async {
    final visible = switch (WidgetsBinding.instance.lifecycleState) {
      null || AppLifecycleState.resumed => true,
      _ => false,
    };
    if (!visible || !_messages.any((message) => !message.outgoing)) {
      return;
    }
    try {
      await widget.messenger.markRead(widget.chat);
    } on MessengerFailure {
      // The next change tries again.
    }
  }

  /// Reloads at least the messages already shown, newest first.
  Future<void> _reload() async {
    final limit = max(_page, _messages.length);
    try {
      final messages = await widget.messenger.messages(
        widget.chat,
        limit: limit,
      );
      if (mounted) {
        setState(() {
          _messages = messages;
          _complete = messages.length < limit;
        });
        unawaited(_markRead());
      }
    } on MessengerFailure {
      // Keep what is shown; the next change tries again.
    }
  }

  Future<void> _loadOlder() async {
    if (_loadingOlder || _complete || _messages.isEmpty) {
      return;
    }
    _loadingOlder = true;
    try {
      final older = await widget.messenger.messages(
        widget.chat,
        before: _messages.last.seq,
      );
      if (mounted) {
        setState(() {
          _messages = [..._messages, ...older];
          _complete = older.length < _page;
        });
      }
    } on MessengerFailure {
      // Try again on the next scroll.
    } finally {
      _loadingOlder = false;
    }
  }

  bool _attaching = false;

  /// Picks a file, prepares it and sends it.
  Future<void> _attach() async {
    final picked = await openFile();
    if (picked == null || !mounted) {
      return;
    }
    setState(() => _attaching = true);
    final say = ScaffoldMessenger.of(context);
    try {
      final bytes = await picked.readAsBytes();
      if (bytes.length > maxFileBytes) {
        say.showSnackBar(
          const SnackBar(content: Text('Files can be up to 25 MB.')),
        );
        return;
      }
      final file = await OutgoingFile.prepare(picked.name, bytes);
      await widget.messenger.sendFile(widget.chat, file);
    } on MessengerFailure catch (failure) {
      say.showSnackBar(SnackBar(content: Text(describeFailure(failure.code))));
    } finally {
      if (mounted) {
        setState(() => _attaching = false);
      }
    }
  }

  Future<void> _details() async {
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) =>
            GroupDetailsScreen(messenger: widget.messenger, chat: widget.chat),
      ),
    );
  }

  /// Removes the chat from this device after the user confirms.
  Future<void> _delete(String title, {required bool request}) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(request ? 'Decline this chat?' : 'Delete this chat?'),
        content: Text(
          'It disappears from this device with all its messages. '
          '$title won’t be told.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          TextButton(
            key: const Key('chat.delete.confirm'),
            onPressed: () => Navigator.of(context).pop(true),
            style: TextButton.styleFrom(foregroundColor: context.colors.danger),
            child: Text(request ? 'Decline' : 'Delete'),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) {
      return;
    }
    try {
      await widget.messenger.deleteChat(widget.chat);
      if (mounted) {
        Navigator.of(context).pop();
      }
    } on MessengerFailure catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(describeFailure(failure.code))));
      }
    }
  }

  Future<void> _accept() async {
    try {
      await widget.messenger.acceptChat(widget.chat);
    } on MessengerFailure catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(describeFailure(failure.code))));
      }
    }
  }

  Future<void> _send(String text) async {
    final replying = _replying;
    final editing = _editing;
    setState(() {
      _replying = null;
      _editing = null;
    });
    final messenger = widget.messenger;
    await _act(
      () => editing != null
          ? messenger.editText(widget.chat, editing.id, text)
          : replying != null
          ? messenger.replyText(widget.chat, replying.id, text)
          : messenger.sendText(widget.chat, text),
    );
  }

  /// Runs [action], explaining a failure in words.
  Future<void> _act(Future<void> Function() action) async {
    try {
      await action();
    } on MessengerFailure catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(describeFailure(failure.code))));
      }
    }
  }

  /// Sets [emoji] as the user's reaction, or removes it if it already is.
  Future<void> _react(MessageView message, String emoji) {
    final mine = message.reactions.where((r) => r.mine).firstOrNull?.emoji;
    return _act(
      () => widget.messenger.react(
        widget.chat,
        message.id,
        mine == emoji ? null : emoji,
      ),
    );
  }

  Future<void> _showActions(MessageView message) async {
    if (message.deleted || message.unsupported) {
      return;
    }
    final action = await showModalBottomSheet<MessageAction>(
      context: context,
      backgroundColor: context.colors.surfaceRaised,
      constraints: const BoxConstraints(maxWidth: 480),
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(ZRadius.card)),
      ),
      builder: (_) => MessageActions(message),
    );
    if (!mounted || action == null) {
      return;
    }
    switch (action) {
      case React(:final emoji):
        await _react(message, emoji);
      case Answer():
        setState(() {
          _replying = message;
          _editing = null;
        });
      case CopyText():
        await Clipboard.setData(ClipboardData(text: message.text));
        if (mounted) {
          ScaffoldMessenger.of(context)
              .showSnackBar(const SnackBar(content: Text('Copied')));
        }
      case Revise():
        setState(() {
          _editing = message;
          _replying = null;
          _input
            ..text = message.text
            ..selection = TextSelection.collapsed(offset: message.text.length);
        });
      case Remove():
        await _confirmDeleteMessage(message);
    }
  }

  Future<void> _confirmDeleteMessage(MessageView message) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Delete for everyone?'),
        content: const Text(
          'The message disappears for everyone in this chat.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          TextButton(
            key: const Key('message.delete.confirm'),
            onPressed: () => Navigator.of(context).pop(true),
            style: TextButton.styleFrom(foregroundColor: context.colors.danger),
            child: const Text('Delete'),
          ),
        ],
      ),
    );
    if (confirmed ?? false) {
      await _act(() => widget.messenger.deleteMessage(widget.chat, message.id));
    }
  }

  Future<void> _safetyCode(String title) async {
    try {
      final groups = await widget.messenger.safetyCode(widget.chat);
      if (groups != null && mounted) {
        await showSafetyCode(context, title: title, groups: groups);
      }
    } on MessengerFailure catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(describeFailure(failure.code))));
      }
    }
  }

  void _cancelMode() {
    setState(() {
      if (_editing != null) {
        _input.clear();
      }
      _replying = null;
      _editing = null;
    });
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;

    return ListenableBuilder(
      listenable: widget.messenger,
      builder: (context, _) {
        final preview = widget.messenger.chats
            .where((chat) => chat.id == widget.chat)
            .firstOrNull;
        final title = preview?.title ?? 'Chat';
        final request = preview?.request ?? false;
        final group = preview?.group ?? false;
        final subtitle = preview != null && !preview.verified
            ? 'Name not yet confirmed by Zelamar'
            : switch (widget.messenger.link) {
                Link.online => 'End-to-end encrypted',
                Link.connecting => 'Connecting…',
                Link.offline => 'Offline: messages will wait',
              };
        return Scaffold(
          appBar: AppBar(
            backgroundColor: colors.surfaceRaised,
            surfaceTintColor: Colors.transparent,
            titleSpacing: 0,
            shape: Border(bottom: BorderSide(color: colors.hairline)),
            actions: [
              PopupMenuButton<void>(
                key: const Key('chat.menu'),
                tooltip: 'More',
                icon: Icon(Icons.more_horiz, color: colors.textSecondary),
                itemBuilder: (_) => [
                  if (group)
                    PopupMenuItem<void>(
                      key: const Key('chat.details'),
                      onTap: () => unawaited(_details()),
                      child: const Text('Group details'),
                    )
                  else
                    PopupMenuItem<void>(
                      key: const Key('chat.safety'),
                      onTap: () => unawaited(_safetyCode(title)),
                      child: const Text('Safety code'),
                    ),
                  if (request || !group)
                    PopupMenuItem<void>(
                      key: const Key('chat.delete'),
                      onTap: () => unawaited(_delete(title, request: request)),
                      child: Text(request ? 'Decline' : 'Delete chat'),
                    ),
                ],
              ),
            ],
            title: Row(
              children: [
                Avatar(title, size: 36),
                const SizedBox(width: ZSpace.md),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        title,
                        key: const Key('chat.title'),
                        style: text.titleMedium,
                        overflow: TextOverflow.ellipsis,
                      ),
                      Text(
                        subtitle,
                        style: text.labelSmall?.copyWith(
                          color: colors.textTertiary,
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
          body: Column(
            children: [
              Expanded(
                child: NotificationListener<ScrollNotification>(
                  onNotification: (notification) {
                    if (notification.metrics.extentAfter < 400) {
                      unawaited(_loadOlder());
                    }
                    return false;
                  },
                  child: ListView.builder(
                    key: const Key('chat.messages'),
                    // Newest at the bottom: the list grows upwards.
                    reverse: true,
                    padding: const EdgeInsets.symmetric(
                      horizontal: ZSpace.md,
                      vertical: ZSpace.lg,
                    ),
                    itemCount: _messages.length,
                    itemBuilder: (context, index) {
                      final message = _messages[index];
                      final older = index + 1 < _messages.length
                          ? _messages[index + 1]
                          : null;
                      final newer = index > 0 ? _messages[index - 1] : null;
                      final startsDay =
                          older == null ||
                          !sameDay(older.sentAtMs, message.sentAtMs);
                      bool together(MessageView? a, MessageView? b) =>
                          a != null &&
                          b != null &&
                          a.event == null &&
                          b.event == null &&
                          a.outgoing == b.outgoing &&
                          a.author == b.author &&
                          sameDay(a.sentAtMs, b.sentAtMs);
                      final event = message.event;
                      return Column(
                        children: [
                          if (startsDay) DayLabel(dayLabel(message.sentAtMs)),
                          if (event != null)
                            EventLine(describeEvent(message.author, event))
                          else
                            MessageBubble(
                              message,
                              grouped: together(message, newer),
                              peer: title,
                              loadFile: () => widget.messenger.openFile(
                                widget.chat,
                                message.id,
                              ),
                              author:
                                  group &&
                                      !message.outgoing &&
                                      !together(message, older)
                                  ? message.author
                                  : null,
                              onActions: () => _showActions(message),
                              onReact: (emoji) => _react(message, emoji),
                            ),
                        ],
                      );
                    },
                  ),
                ),
              ),
              if (request)
                RequestBar(
                  title: title,
                  onAccept: _accept,
                  onDecline: () => _delete(title, request: true),
                ),
              Composer(
                controller: _input,
                onSend: _send,
                onCancel: _cancelMode,
                onAttach: _attaching ? null : _attach,
                attaching: _attaching,
                banner: switch ((_editing, _replying)) {
                  (final editing?, _) => ComposerBanner(
                    icon: Icons.edit_outlined,
                    label: 'Editing',
                    text: editing.text,
                    onClose: _cancelMode,
                  ),
                  (_, final replying?) => ComposerBanner(
                    icon: Icons.reply_rounded,
                    label: replying.outgoing
                        ? 'Replying to yourself'
                        : 'Replying to $title',
                    text: replying.text,
                    onClose: _cancelMode,
                  ),
                  _ => null,
                },
              ),
            ],
          ),
        );
      },
    );
  }
}

/// Shown in a chat someone else started, until the user answers.
class RequestBar extends StatelessWidget {
  const new({
    required this.title,
    required this.onAccept,
    required this.onDecline,
    super.key,
  });

  final String title;
  final VoidCallback onAccept;
  final VoidCallback onDecline;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    return Container(
      key: const Key('chat.request'),
      width: double.infinity,
      decoration: BoxDecoration(
        color: colors.surface,
        border: Border(top: BorderSide(color: colors.hairline)),
      ),
      padding: const EdgeInsets.fromLTRB(
        ZSpace.gutter,
        ZSpace.lg,
        ZSpace.gutter,
        ZSpace.md,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('$title wants to chat with you.', style: text.titleSmall),
          const SizedBox(height: ZSpace.xs),
          Text(
            'They won’t know you’ve seen this until you reply or accept.',
            style: text.bodySmall,
          ),
          const SizedBox(height: ZSpace.md),
          Row(
            children: [
              Expanded(
                child: OutlinedButton(
                  key: const Key('chat.decline'),
                  onPressed: onDecline,
                  style: OutlinedButton.styleFrom(
                    foregroundColor: colors.danger,
                  ),
                  child: const Text('Decline'),
                ),
              ),
              const SizedBox(width: ZSpace.md),
              Expanded(
                child: FilledButton(
                  key: const Key('chat.accept'),
                  onPressed: onAccept,
                  child: const Text('Accept'),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

/// A quiet separator between days.
class DayLabel extends StatelessWidget {
  const new(this.label, {super.key});

  final String label;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: ZSpace.lg),
      child: Text(
        label,
        style: context.text.labelSmall?.copyWith(
          color: colors.textTertiary,
          letterSpacing: 0.4,
        ),
      ),
    );
  }
}

/// One message: sand on the right for the user's own, white on the left
/// for others, the small corner pointing at the sender.
/// Something that happened to the chat, between the messages.
class EventLine extends StatelessWidget {
  const new(this.label, {super.key});

  final String label;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: ZSpace.md),
      child: Center(
        child: Text(
          label,
          key: const Key('chat.event'),
          textAlign: TextAlign.center,
          style: context.text.labelMedium?.copyWith(
            color: context.colors.textTertiary,
          ),
        ),
      ),
    );
  }
}

class MessageBubble extends StatelessWidget {
  const new(
    this.message, {
    this.grouped = false,
    this.peer = '',
    this.author,
    this.loadFile,
    this.onActions,
    this.onReact,
    super.key,
  });

  final MessageView message;

  /// Fetches and opens the message's file, if it has one.
  final FileLoader? loadFile;

  /// Who wrote it, shown above the text: in groups, for the first of a run.
  final String? author;

  /// Followed by a newer message from the same side: less space below.
  final bool grouped;

  /// The other side's name, for quotes of their messages.
  final String peer;

  /// Opens the message's actions (long press, right click).
  final VoidCallback? onActions;

  /// Toggles one reaction (tap on a reaction under the bubble).
  final ValueChanged<String>? onReact;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    final outgoing = message.outgoing;
    final meta = outgoing ? colors.bubbleOutgoingMeta : colors.textTertiary;
    final quiet = message.deleted || message.unsupported;
    final body = message.deleted
        ? 'Message deleted'
        : message.unsupported
        ? 'Update Zelamar to see this message'
        : message.text;
    const round = Radius.circular(ZRadius.bubble);
    const tail = Radius.circular(ZRadius.bubbleTail);

    final quote = message.replyTo;
    final bubble = GestureDetector(
      onLongPress: onActions,
      onSecondaryTap: onActions,
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: min(MediaQuery.sizeOf(context).width * 0.78, 520),
        ),
        child: Container(
          key: Key('chat.message.${message.id}'),
          padding: const EdgeInsets.fromLTRB(14, 9, 14, 7),
          decoration: BoxDecoration(
            color: outgoing ? colors.bubbleOutgoing : colors.bubbleIncoming,
            border: outgoing
                ? null
                : Border.all(color: colors.bubbleIncomingBorder),
            borderRadius: BorderRadius.only(
              topLeft: round,
              topRight: round,
              bottomLeft: outgoing ? round : tail,
              bottomRight: outgoing ? tail : round,
            ),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.end,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (author != null)
                Align(
                  alignment: Alignment.centerLeft,
                  widthFactor: 1,
                  child: Padding(
                    padding: const EdgeInsets.only(bottom: 2),
                    child: Text(
                      author!,
                      key: const Key('chat.author'),
                      style: text.labelMedium?.copyWith(color: colors.accent),
                    ),
                  ),
                ),
              if (quote != null && !message.deleted)
                QuoteBox(quote, author: quote.outgoing ? 'You' : peer),
              if (message.file case final file? when !message.deleted)
                Padding(
                  padding: EdgeInsets.only(
                    bottom: message.text.isEmpty ? ZSpace.xxs : ZSpace.xs,
                  ),
                  child: file.mime.startsWith('image/') && loadFile != null
                      ? ImageAttachment(file: file, load: loadFile!)
                      : FileAttachment(
                          file: file,
                          load: loadFile ?? () async => Uint8List(0),
                          outgoing: outgoing,
                        ),
                ),
              Align(
                alignment: Alignment.centerLeft,
                widthFactor: 1,
                child: Text(
                  body,
                  style: text.bodyLarge?.copyWith(
                    color: quiet
                        ? meta
                        : outgoing
                        ? colors.onBubbleOutgoing
                        : colors.textPrimary,
                    fontStyle: quiet ? FontStyle.italic : null,
                  ),
                ),
              ),
              const SizedBox(height: ZSpace.xxs),
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  if (message.edited)
                    Text(
                      'edited · ',
                      style: text.labelSmall?.copyWith(color: meta),
                    ),
                  Text(
                    clockTime(message.sentAtMs),
                    style: text.labelSmall?.copyWith(color: meta),
                  ),
                  if (outgoing) ...[
                    const SizedBox(width: ZSpace.xs),
                    StatusIcon(message.status, color: meta),
                  ],
                ],
              ),
            ],
          ),
        ),
      ),
    );
    return Padding(
      padding: EdgeInsets.only(bottom: grouped ? ZSpace.xxs : ZSpace.sm),
      child: Column(
        crossAxisAlignment: outgoing
            ? CrossAxisAlignment.end
            : CrossAxisAlignment.start,
        children: [
          bubble,
          if (message.reactions.isNotEmpty && !message.deleted)
            Padding(
              padding: const EdgeInsets.only(top: ZSpace.xs),
              child: Wrap(
                spacing: ZSpace.xs,
                children: [
                  for (final reaction in message.reactions)
                    ReactionChip(
                      reaction,
                      onTap: onReact == null
                          ? null
                          : () => onReact!(reaction.emoji),
                    ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

/// The message a reply answers, quietly, at the top of the bubble.
class QuoteBox extends StatelessWidget {
  const new(this.quote, {required this.author, super.key});

  final Quote quote;
  final String author;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    return Container(
      key: const Key('chat.quote'),
      margin: const EdgeInsets.only(bottom: ZSpace.xs),
      padding: const EdgeInsets.fromLTRB(
        ZSpace.sm,
        ZSpace.xs,
        ZSpace.sm,
        ZSpace.xs,
      ),
      decoration: BoxDecoration(
        color: colors.surfaceSunken.withValues(alpha: 0.6),
        border: Border(left: BorderSide(color: colors.accent, width: 2)),
        borderRadius: BorderRadius.circular(ZSpace.xs),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            author,
            style: text.labelSmall?.copyWith(
              color: colors.accent,
              fontWeight: FontWeight.w600,
            ),
          ),
          Text(
            quote.text,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: text.bodySmall,
          ),
        ],
      ),
    );
  }
}

/// One reaction under a message: bronze when it is the user's own.
class ReactionChip extends StatelessWidget {
  const new(this.reaction, {this.onTap, super.key});

  final Reaction reaction;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Material(
      color: reaction.mine ? colors.bubbleOutgoing : colors.surface,
      shape: StadiumBorder(
        side: BorderSide(
          color: reaction.mine ? colors.accent : colors.hairline,
        ),
      ),
      child: InkWell(
        key: Key('chat.reaction.${reaction.emoji}'),
        customBorder: const StadiumBorder(),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
          child: Text(
            reaction.count > 1
                ? '${reaction.emoji} ${reaction.count}'
                : reaction.emoji,
            style: context.text.bodySmall?.copyWith(color: colors.textPrimary),
          ),
        ),
      ),
    );
  }
}

/// What the user chose in a message's actions.
sealed class MessageAction {
  const new();
}

final class React extends MessageAction {
  const new(this.emoji);

  final String emoji;
}

final class Answer extends MessageAction {
  const new();
}

final class CopyText extends MessageAction {
  const new();
}

final class Revise extends MessageAction {
  const new();
}

final class Remove extends MessageAction {
  const new();
}

/// Reactions offered for every message.
const quickReactions = ['❤️', '👍', '😂', '😮', '😢', '🙏'];

/// A message's actions: reactions first, then what can be done with it.
class MessageActions extends StatelessWidget {
  const new(this.message, {super.key});

  final MessageView message;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final mine = message.reactions.where((r) => r.mine).firstOrNull?.emoji;
    void choose(MessageAction action) => Navigator.of(context).pop(action);
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: ZSpace.lg),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: ZSpace.lg),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                children: [
                  for (final emoji in quickReactions)
                    InkWell(
                      key: Key('actions.react.$emoji'),
                      customBorder: const CircleBorder(),
                      onTap: () => choose(React(emoji)),
                      child: Container(
                        width: 48,
                        height: 48,
                        alignment: Alignment.center,
                        decoration: BoxDecoration(
                          shape: BoxShape.circle,
                          color: emoji == mine ? colors.bubbleOutgoing : null,
                          border: emoji == mine
                              ? Border.all(color: colors.accent)
                              : null,
                        ),
                        child: Text(
                          emoji,
                          style: const TextStyle(fontSize: 24),
                        ),
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(height: ZSpace.md),
            const Divider(),
            ListTile(
              key: const Key('actions.reply'),
              leading: const Icon(Icons.reply_rounded),
              title: const Text('Reply'),
              onTap: () => choose(const Answer()),
            ),
            ListTile(
              key: const Key('actions.copy'),
              leading: const Icon(Icons.copy_rounded),
              title: const Text('Copy'),
              onTap: () => choose(const CopyText()),
            ),
            if (message.editable) ...[
              ListTile(
                key: const Key('actions.edit'),
                leading: const Icon(Icons.edit_outlined),
                title: const Text('Edit'),
                onTap: () => choose(const Revise()),
              ),
              ListTile(
                key: const Key('actions.delete'),
                leading: Icon(Icons.delete_outline, color: colors.danger),
                title: Text(
                  'Delete for everyone',
                  style: TextStyle(color: colors.danger),
                ),
                onTap: () => choose(const Remove()),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// Above the composer: what the next message answers or edits.
class ComposerBanner extends StatelessWidget {
  const new({
    required this.icon,
    required this.label,
    required this.text,
    required this.onClose,
    super.key,
  });

  final IconData icon;
  final String label;
  final String text;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final style = context.text;
    return Padding(
      key: const Key('chat.banner'),
      padding: const EdgeInsets.only(bottom: ZSpace.sm),
      child: Row(
        children: [
          Icon(icon, size: 18, color: colors.accent),
          const SizedBox(width: ZSpace.sm),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  label,
                  style: style.labelSmall?.copyWith(
                    color: colors.accent,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                Text(
                  text,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: style.bodySmall,
                ),
              ],
            ),
          ),
          IconButton(
            key: const Key('chat.banner.close'),
            tooltip: 'Cancel',
            onPressed: onClose,
            icon: Icon(
              Icons.close_rounded,
              size: 18,
              color: colors.textTertiary,
            ),
          ),
        ],
      ),
    );
  }
}

/// Clock while waiting, a check when sent, two when delivered, bronze when
/// read.
class StatusIcon extends StatelessWidget {
  const new(this.status, {required this.color, super.key});

  final MessageStatus status;
  final Color color;

  @override
  Widget build(BuildContext context) {
    final (icon, tint, label) = switch (status) {
      MessageStatus.queued => (
        Icons.schedule_rounded,
        color,
        'Waiting to send',
      ),
      MessageStatus.sent => (Icons.check_rounded, color, 'Sent'),
      MessageStatus.delivered => (Icons.done_all_rounded, color, 'Delivered'),
      MessageStatus.read => (
        Icons.done_all_rounded,
        context.colors.accent,
        'Read',
      ),
    };
    return Icon(
      icon,
      key: Key('chat.status.${status.name}'),
      size: 14,
      color: tint,
      semanticLabel: label,
    );
  }
}

/// Writing a message. Enter sends; Shift+Enter starts a new line.
class Composer extends StatefulWidget {
  const new({
    required this.controller,
    required this.onSend,
    this.banner,
    this.onCancel,
    this.onAttach,
    this.attaching = false,
    super.key,
  });

  final TextEditingController controller;
  final Future<void> Function(String text) onSend;

  /// Picks a file to send; `null` while one is on its way.
  final VoidCallback? onAttach;

  /// A file is being prepared and uploaded.
  final bool attaching;

  /// What the next message answers or edits, shown above the field.
  final Widget? banner;

  /// Escape: stop answering or editing.
  final VoidCallback? onCancel;

  @override
  State<Composer> createState() => _ComposerState();
}

class _ComposerState extends State<Composer> {
  TextEditingController get _text => widget.controller;
  late final _focus = FocusNode(onKeyEvent: _onKey);

  bool get _canSend => _text.text.trim().isNotEmpty;

  @override
  void initState() {
    super.initState();
    _text.addListener(_changed);
  }

  @override
  void didUpdateWidget(Composer old) {
    super.didUpdateWidget(old);
    if (old.controller != widget.controller) {
      old.controller.removeListener(_changed);
      widget.controller.addListener(_changed);
    }
    if (old.banner == null && widget.banner != null) {
      _focus.requestFocus();
    }
  }

  @override
  void dispose() {
    _text.removeListener(_changed);
    _focus.dispose();
    super.dispose();
  }

  void _changed() => setState(() {});

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    final enter =
        event.logicalKey == LogicalKeyboardKey.enter ||
        event.logicalKey == LogicalKeyboardKey.numpadEnter;
    if (enter &&
        event is KeyDownEvent &&
        !HardwareKeyboard.instance.isShiftPressed) {
      unawaited(_send());
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.escape &&
        event is KeyDownEvent &&
        widget.banner != null) {
      widget.onCancel?.call();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  Future<void> _send() async {
    final text = _text.text.trim();
    if (text.isEmpty) {
      return;
    }
    _text.clear();
    await widget.onSend(text);
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Container(
      decoration: BoxDecoration(
        color: colors.surfaceRaised,
        border: Border(top: BorderSide(color: colors.hairline)),
      ),
      padding: EdgeInsets.fromLTRB(
        ZSpace.md,
        ZSpace.sm,
        ZSpace.md,
        ZSpace.sm + MediaQuery.paddingOf(context).bottom,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ?widget.banner,
          Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              SizedBox(
                width: 44,
                height: 44,
                child: widget.attaching
                    ? Padding(
                        padding: const EdgeInsets.all(12),
                        child: CircularProgressIndicator(
                          strokeWidth: 2,
                          color: colors.accent,
                        ),
                      )
                    : IconButton(
                        key: const Key('chat.attach'),
                        tooltip: 'Attach a photo or file',
                        onPressed: widget.onAttach,
                        icon: Icon(
                          Icons.attach_file_rounded,
                          color: colors.textSecondary,
                        ),
                      ),
              ),
              const SizedBox(width: ZSpace.xs),
              Expanded(
                child: TextField(
                  key: const Key('chat.input'),
                  controller: _text,
                  focusNode: _focus,
                  minLines: 1,
                  maxLines: 6,
                  keyboardType: TextInputType.multiline,
                  textCapitalization: TextCapitalization.sentences,
                  decoration: const InputDecoration(hintText: 'Message'),
                  style: context.text.bodyLarge,
                ),
              ),
              const SizedBox(width: ZSpace.sm),
              AnimatedContainer(
                duration: ZMotion.fast,
                width: 44,
                height: 44,
                decoration: BoxDecoration(
                  color: _canSend ? colors.accent : colors.surfaceSunken,
                  shape: BoxShape.circle,
                ),
                child: IconButton(
                  key: const Key('chat.send'),
                  tooltip: 'Send',
                  onPressed: _canSend ? _send : null,
                  icon: Icon(
                    Icons.arrow_upward_rounded,
                    color: _canSend ? colors.onAccent : colors.textTertiary,
                  ),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
