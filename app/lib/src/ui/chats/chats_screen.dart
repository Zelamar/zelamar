import 'package:flutter/material.dart';
import 'package:zelamar/src/design/emblem.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/avatar.dart';
import 'package:zelamar/src/ui/chat/chat_screen.dart';
import 'package:zelamar/src/ui/chats/new_chat_sheet.dart';
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
                                      '@$name',
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
                      IconButton.filled(
                        key: const Key('chats.new'),
                        tooltip: 'New chat',
                        onPressed: () => _newChat(context),
                        icon: const Icon(Icons.edit_outlined, size: 20),
                      ),
                    ],
                  ),
                ),
                Expanded(
                  child: chats.isEmpty
                      ? _EmptyChats(onStart: () => _newChat(context))
                      : ListView.separated(
                          padding: const EdgeInsets.only(bottom: ZSpace.xl),
                          itemCount: chats.length,
                          separatorBuilder: (_, _) => const Padding(
                            padding: EdgeInsets.only(left: 92),
                            child: Divider(),
                          ),
                          itemBuilder: (context, index) => ChatTile(
                            chats[index],
                            onTap: () => _open(context, chats[index].id),
                          ),
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
      Link.offline => (colors.textTertiary, 'Offline: messages will wait'),
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
                      Flexible(
                        child: Text(
                          chat.title,
                          style: text.titleMedium,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      if (!chat.verified) ...[
                        const SizedBox(width: ZSpace.sm),
                        Text(
                          'unconfirmed',
                          style: text.labelSmall?.copyWith(
                            color: colors.textTertiary,
                          ),
                        ),
                      ],
                      const Spacer(),
                      if (last != null)
                        Text(listTime(last.sentAtMs), style: text.labelSmall),
                    ],
                  ),
                  const SizedBox(height: ZSpace.xs),
                  Text(
                    preview(last),
                    style: text.bodyMedium,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
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

/// The one-line summary of a chat's newest message.
String preview(MessageView? last) {
  if (last == null) {
    return 'New chat';
  }
  final body = last.deleted
      ? 'Message deleted'
      : last.unsupported
      ? 'Update Zelamar to see this message'
      : last.text.replaceAll('\n', ' ');
  return last.outgoing ? 'You: $body' : body;
}

class _EmptyChats extends StatelessWidget {
  const new({required this.onStart});

  final VoidCallback onStart;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(ZSpace.gutter),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const SunsetZEmblem(size: 72),
            const SizedBox(height: ZSpace.xl),
            Text('Your conversations live here.', style: text.titleMedium),
            const SizedBox(height: ZSpace.sm),
            Text(
              'Only you and the people you write to can read them.',
              style: text.bodyMedium?.copyWith(color: colors.textSecondary),
              textAlign: TextAlign.center,
            ),
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
