import 'package:flutter/material.dart';
import 'package:zelamar/src/design/emblem.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/avatar.dart';
import 'package:zelamar/src/ui/chat/chat_screen.dart';
import 'package:zelamar/src/ui/chats/new_chat_sheet.dart';
import 'package:zelamar/src/ui/profile/profile_screen.dart';
import 'package:zelamar/src/ui/profile/recovery_key_screen.dart';
import 'package:zelamar/src/ui/words.dart';

/// Every conversation, most recent first.
class ChatsScreen extends StatelessWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  Future<void> _open(BuildContext context, String chat) =>
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChatScreen(messenger: messenger, chat: chat),
        ),
      );

  Future<void> _newChat(BuildContext context) async {
    final chat = await showNewChatSheet(context, messenger);
    if (chat != null && context.mounted) {
      await _open(context, chat);
    }
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: messenger,
      builder: (context, _) {
        final colors = context.colors;
        final text = context.text;
        final chats = messenger.chats;
        return Scaffold(
          body: SafeArea(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(
                    ZSpace.gutter,
                    ZSpace.xl,
                    ZSpace.lg,
                    ZSpace.md,
                  ),
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.end,
                    children: [
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text('Chats', style: text.displayMedium),
                            if (messenger.username case final name?)
                              Padding(
                                padding: const EdgeInsets.only(top: ZSpace.xs),
                                child: Row(
                                  children: [
                                    LinkDot(messenger.link),
                                    const SizedBox(width: ZSpace.sm),
                                    Text(
                                      messenger.link == Link.online
                                          ? '@$name'
                                          : '@$name · Connecting…',
                                      key: const Key('chats.username'),
                                      style: text.bodySmall?.copyWith(
                                        color: colors.textTertiary,
                                      ),
                                    ),
                                  ],
                                ),
                              ),
                          ],
                        ),
                      ),
                      IconButton(
                        key: const Key('chats.profile'),
                        tooltip: 'You',
                        onPressed: () => Navigator.of(context).push(
                          MaterialPageRoute<void>(
                            builder: (_) => ProfileScreen(messenger: messenger),
                          ),
                        ),
                        icon: Icon(
                          Icons.person_outline,
                          color: colors.textSecondary,
                        ),
                      ),
                      const SizedBox(width: ZSpace.xs),
                      IconButton.filled(
                        key: const Key('chats.new'),
                        tooltip: 'New chat',
                        onPressed: () => _newChat(context),
                        icon: const Icon(Icons.edit_outlined, size: 20),
                      ),
                    ],
                  ),
                ),
                if (messenger.canLinkDevices && messenger.recoveryKey == null)
                  _RecoveryReminder(
                    onTap: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => RecoveryKeyScreen(messenger: messenger),
                      ),
                    ),
                  ),
                Expanded(
                  child: chats.isEmpty
                      ? _EmptyChats(onStart: () => _newChat(context))
                      : ListView(
                          padding: const EdgeInsets.only(bottom: ZSpace.xl),
                          children: [
                            ..._section(
                              context,
                              'Requests',
                              chats.where((chat) => chat.request).toList(),
                              labelled: true,
                            ),
                            ..._section(
                              context,
                              'Chats',
                              chats.where((chat) => !chat.request).toList(),
                              labelled: chats.any((chat) => chat.request),
                            ),
                          ],
                        ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

extension on ChatsScreen {
  /// [chats] as tiles, under a quiet section label when [labelled].
  List<Widget> _section(
    BuildContext context,
    String label,
    List<ChatPreview> chats, {
    required bool labelled,
  }) {
    if (chats.isEmpty) {
      return const [];
    }
    return [
      if (labelled)
        Padding(
          padding: const EdgeInsets.fromLTRB(
            ZSpace.gutter,
            ZSpace.lg,
            ZSpace.gutter,
            ZSpace.xs,
          ),
          child: Text(
            label.toUpperCase(),
            key: Key('chats.section.${label.toLowerCase()}'),
            style: context.text.labelMedium?.copyWith(
              color: context.colors.textTertiary,
            ),
          ),
        ),
      for (final (index, chat) in chats.indexed) ...[
        if (index > 0)
          const Padding(padding: EdgeInsets.only(left: 92), child: Divider()),
        ChatTile(chat, onTap: () => _open(context, chat.id)),
      ],
    ];
  }
}

/// A quiet reminder, until there is a recovery key, that losing this device
/// would lose the account (ADR-0018).
class _RecoveryReminder extends StatelessWidget {
  const new({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        ZSpace.gutter,
        ZSpace.xs,
        ZSpace.gutter,
        ZSpace.md,
      ),
      child: Material(
        color: colors.surface,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(ZRadius.field),
          side: BorderSide(color: colors.hairline),
        ),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          key: const Key('chats.recovery'),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: ZSpace.lg,
              vertical: ZSpace.md,
            ),
            child: Row(
              children: [
                Icon(Icons.key_outlined, color: colors.accent),
                const SizedBox(width: ZSpace.md),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('Save your recovery key', style: text.titleSmall),
                      const SizedBox(height: ZSpace.xxs),
                      Text(
                        'In case you lose this device.',
                        style: text.bodySmall,
                      ),
                    ],
                  ),
                ),
                Icon(Icons.chevron_right, color: colors.textTertiary),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// A small dot for the connection: olive when live, bronze while
/// connecting, quiet when offline (messages still wait in the outbox).
class LinkDot extends StatelessWidget {
  const new(this.link, {super.key});

  final Link link;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final (color, label) = switch (link) {
      Link.online => (colors.online, 'Connected'),
      Link.connecting => (colors.accent, 'Connecting…'),
      Link.offline => (
        colors.textTertiary,
        'Offline. Messages send when you’re back online.',
      ),
    };
    return Tooltip(
      message: label,
      child: AnimatedContainer(
        duration: ZMotion.base,
        width: 7,
        height: 7,
        decoration: BoxDecoration(color: color, shape: BoxShape.circle),
      ),
    );
  }
}

/// One chat in the list.
class ChatTile extends StatelessWidget {
  const new(this.chat, {required this.onTap, super.key});

  final ChatPreview chat;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    final last = chat.last;
    final unread = chat.unread > 0;
    return InkWell(
      key: Key('chats.tile.${chat.id}'),
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: ZSpace.gutter,
          vertical: ZSpace.md,
        ),
        child: Row(
          children: [
            Avatar(chat.title, size: 52),
            const SizedBox(width: ZSpace.lg),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Expanded(
                        child: Text(
                          chat.title,
                          style: text.titleMedium,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      if (last != null)
                        Text(
                          listTime(last.sentAtMs),
                          style: text.labelSmall?.copyWith(
                            color: unread ? colors.accent : null,
                          ),
                        ),
                    ],
                  ),
                  const SizedBox(height: ZSpace.xs),
                  Row(
                    children: [
                      Expanded(
                        child: Text(
                          preview(last, group: chat.group),
                          style: text.bodyMedium?.copyWith(
                            color: unread ? colors.textPrimary : null,
                          ),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      if (unread) ...[
                        const SizedBox(width: ZSpace.sm),
                        UnreadBadge(chat.unread),
                      ],
                    ],
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// A bronze pill with the number of unread messages.
class UnreadBadge extends StatelessWidget {
  const new(this.count, {super.key});

  final int count;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Container(
      key: const Key('chats.unread'),
      constraints: const BoxConstraints(minWidth: 20),
      height: 20,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: colors.accent,
        borderRadius: BorderRadius.circular(10),
      ),
      child: Text(
        count >= 99 ? '99+' : '$count',
        style: context.text.labelSmall?.copyWith(
          color: colors.onAccent,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}

/// The one-line summary of a chat's newest message.
String preview(MessageView? last, {bool group = false}) {
  if (last == null) {
    return 'New chat';
  }
  final event = last.event;
  if (event != null) {
    return describeEvent(last.author, event);
  }
  final call = last.call;
  if (call != null) {
    final icon = call.video ? '📹' : '📞';
    return '$icon ${callLine(call, outgoing: last.outgoing)}';
  }
  final file = last.file;
  final body = last.deleted
      ? 'Message deleted'
      : file != null
      ? fileLine(file)
      : last.unsupported
      ? 'Update Zelamar to see this message'
      : last.text.replaceAll('\n', ' ');
  if (last.outgoing) {
    return 'You: $body';
  }
  return group ? '${last.author}: $body' : body;
}

class _EmptyChats extends StatelessWidget {
  const new({required this.onStart});

  final VoidCallback onStart;

  @override
  Widget build(BuildContext context) {
    final text = context.text;
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(ZSpace.gutter),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const SunsetZEmblem(size: 72),
            const SizedBox(height: ZSpace.xl),
            Text('No conversations yet', style: text.titleMedium),
            const SizedBox(height: ZSpace.xl),
            OutlinedButton(
              key: const Key('chats.start'),
              onPressed: onStart,
              child: const Text('Start a conversation'),
            ),
          ],
        ),
      ),
    );
  }
}
