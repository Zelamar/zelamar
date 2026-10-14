import 'dart:async';
import 'dart:math';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/avatar.dart';
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

class _ChatScreenState extends State<ChatScreen> {
  static const _page = 60;

  List<MessageView> _messages = const [];
  bool _loadingOlder = false;
  bool _complete = false;
  StreamSubscription<String?>? _changes;

  @override
  void initState() {
    super.initState();
    _changes = widget.messenger.chatChanged.listen((chat) {
      if (chat == null || chat == widget.chat) {
        unawaited(_reload());
      }
    });
    unawaited(_reload());
  }

  @override
  void dispose() {
    unawaited(_changes?.cancel());
    super.dispose();
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

  Future<void> _send(String text) async {
    try {
      await widget.messenger.sendText(widget.chat, text);
    } on MessengerFailure catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(describeFailure(failure.code))));
      }
    }
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
                      final grouped =
                          newer != null &&
                          newer.outgoing == message.outgoing &&
                          sameDay(newer.sentAtMs, message.sentAtMs);
                      return Column(
                        children: [
                          if (startsDay) DayLabel(dayLabel(message.sentAtMs)),
                          MessageBubble(message, grouped: grouped),
                        ],
                      );
                    },
                  ),
                ),
              ),
              Composer(onSend: _send),
            ],
          ),
        );
      },
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
class MessageBubble extends StatelessWidget {
  const new(this.message, {this.grouped = false, super.key});

  final MessageView message;

  /// Followed by a newer message from the same side: less space below.
  final bool grouped;

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

    return Align(
      alignment: outgoing ? Alignment.centerRight : Alignment.centerLeft,
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: min(MediaQuery.sizeOf(context).width * 0.78, 520),
        ),
        child: Container(
          key: Key('chat.message.${message.id}'),
          margin: EdgeInsets.only(bottom: grouped ? ZSpace.xxs : ZSpace.sm),
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
              Align(
                alignment: Alignment.centerLeft,
                widthFactor: 1,
                child: SelectableText(
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
  const new({required this.onSend, super.key});

  final Future<void> Function(String text) onSend;

  @override
  State<Composer> createState() => _ComposerState();
}

class _ComposerState extends State<Composer> {
  final _text = TextEditingController();
  late final _focus = FocusNode(onKeyEvent: _onKey);

  bool get _canSend => _text.text.trim().isNotEmpty;

  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

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
    return KeyEventResult.ignored;
  }

  Future<void> _send() async {
    final text = _text.text.trim();
    if (text.isEmpty) {
      return;
    }
    _text.clear();
    setState(() {});
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
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
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
              onChanged: (_) => setState(() {}),
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
    );
  }
}
