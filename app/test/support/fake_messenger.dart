import 'dart:async';

import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';

/// A messenger without network or native code, for widget tests. It keeps
/// everything in memory and records what the screens asked of it.
class FakeMessenger extends Messenger {
  new({this.phaseValue = MessengerPhase.needsAccount, this.usernameValue});

  MessengerPhase phaseValue;
  String? usernameValue;
  Link linkValue = Link.online;
  final Map<String, ChatPreview> _chats = {};
  final Map<String, List<MessageView>> conversations = {};
  final _changes = StreamController<String?>.broadcast();

  /// Thrown by the next call that talks to the service.
  MessengerFailure? failNext;

  /// Every text sent, as `chat: text`.
  final sent = <String>[];

  int _seq = 0;

  @override
  MessengerPhase get phase => phaseValue;

  @override
  MessengerFailure? get failure => null;

  @override
  String? get username => usernameValue;

  @override
  Link get link => linkValue;

  @override
  List<ChatPreview> get chats => _chats.values.toList().reversed.toList();

  @override
  Stream<String?> get chatChanged => _changes.stream;

  @override
  String? normalizeUsername(String username) {
    final ok =
        RegExp(r'^[A-Za-z][A-Za-z0-9_]{2,31}$').hasMatch(username) &&
        !username.contains('__') &&
        !username.endsWith('_');
    return ok ? username.toLowerCase() : null;
  }

  void _maybeFail() {
    final failure = failNext;
    if (failure != null) {
      failNext = null;
      throw failure;
    }
  }

  @override
  Future<void> createAccount({
    required String username,
    required String invite,
  }) async {
    _maybeFail();
    usernameValue = username;
    phaseValue = MessengerPhase.ready;
    notifyListeners();
  }

  /// Adds a chat as if someone had started it.
  void addChat(
    String id,
    String title, {
    bool verified = true,
    List<MessageView> messages = const [],
    int unread = 0,
    bool request = false,
  }) {
    conversations[id] = [...messages];
    _chats[id] = ChatPreview(
      id: id,
      title: title,
      verified: verified,
      last: messages.firstOrNull,
      unread: unread,
      request: request,
    );
    notifyListeners();
  }

  @override
  Future<String> startChat(String username) async {
    _maybeFail();
    final id = 'chat-$username';
    addChat(id, '@$username');
    return id;
  }

  @override
  Future<List<MessageView>> messages(
    String chat, {
    int? before,
    int limit = 60,
  }) async {
    final all = conversations[chat] ?? const [];
    return all
        .where((message) => before == null || message.seq < before)
        .take(limit)
        .toList();
  }

  @override
  Future<void> sendText(String chat, String text) async {
    sent.add('$chat: $text');
    final message = view(
      ++_seq + 1000,
      text,
      outgoing: true,
      status: MessageStatus.queued,
    );
    conversations[chat] = [message, ...?conversations[chat]];
    final preview = _chats[chat]!;
    // Writing in a chat accepts it.
    _chats[chat] = ChatPreview(
      id: chat,
      title: preview.title,
      verified: preview.verified,
      last: message,
      unread: preview.unread,
      request: false,
    );
    _changes.add(chat);
    notifyListeners();
  }

  /// Chats marked read, in order.
  final read = <String>[];

  @override
  Future<void> acceptChat(String chat) async {
    final preview = _chats[chat]!;
    _chats[chat] = _copy(preview, request: false);
    notifyListeners();
  }

  @override
  Future<void> deleteChat(String chat) async {
    _chats.remove(chat);
    conversations.remove(chat);
    notifyListeners();
  }

  ChatPreview _copy(ChatPreview preview, {bool? request, int? unread}) =>
      ChatPreview(
        id: preview.id,
        title: preview.title,
        verified: preview.verified,
        last: preview.last,
        unread: unread ?? preview.unread,
        request: request ?? preview.request,
      );

  @override
  Future<void> markRead(String chat) async {
    read.add(chat);
    final preview = _chats[chat];
    if (preview != null && preview.unread > 0) {
      _chats[chat] = _copy(preview, unread: 0);
      notifyListeners();
    }
  }

  @override
  void dispose() {
    unawaited(_changes.close());
    super.dispose();
  }
}

/// A message for tests: [seq] orders it, [sentAtMs] defaults to now.
MessageView view(
  int seq,
  String text, {
  bool outgoing = false,
  MessageStatus status = MessageStatus.sent,
  int? sentAtMs,
  bool deleted = false,
  bool edited = false,
}) => MessageView(
  id: 'm$seq',
  seq: seq,
  outgoing: outgoing,
  text: text,
  deleted: deleted,
  unsupported: false,
  edited: edited,
  sentAtMs: sentAtMs ?? DateTime.now().millisecondsSinceEpoch,
  status: status,
);
