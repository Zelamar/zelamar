import 'package:flutter/foundation.dart';
import 'package:zelamar/src/rust/api/session.dart';

/// Where the app is in its life.
enum MessengerPhase {
  /// Opening the encrypted store.
  starting,

  /// No account on this device yet: show onboarding.
  needsAccount,

  /// Signed in: show chats.
  ready,

  /// The store could not be opened; see [Messenger.failure].
  failed,
}

/// The connection to the delivery service.
enum Link {
  /// No connection; messages wait in the outbox.
  offline,

  /// Trying to connect.
  connecting,

  /// Connected: messages arrive live.
  online,
}

/// A failure with a stable code (see `describeFailure`).
class MessengerFailure implements Exception {
  const new(this.code, [this.detail = '']);

  /// Stable code: the service's (`username_taken`, ...), the core's
  /// (`unauthenticated`, ...), or `offline`.
  final String code;

  /// Details for logs; never shown as is.
  final String detail;

  @override
  String toString() =>
      'MessengerFailure($code${detail.isEmpty ? '' : ': $detail'})';
}

/// Everything the screens need from the messenger, and nothing more.
///
/// The live implementation talks to the Rust core and the delivery
/// service; tests use a fake. Screens listen to it (it is a
/// [ChangeNotifier]) and to [chatChanged] for message updates.
abstract class Messenger extends ChangeNotifier {
  MessengerPhase get phase;

  /// Why [phase] is [MessengerPhase.failed].
  MessengerFailure? get failure;

  /// The account's username, once claimed.
  String? get username;

  Link get link;

  /// Every chat, most recently active first.
  List<ChatPreview> get chats;

  /// A chat id whenever that chat's messages change; `null` when any may
  /// have (for example, a batch of messages was sent).
  Stream<String?> get chatChanged;

  /// Lowercased [username] if it is well-formed, else `null`.
  String? normalizeUsername(String username);

  /// Claims [username] with a beta [invite] and registers this device.
  Future<void> createAccount({
    required String username,
    required String invite,
  });

  /// Starts a chat with [username]; returns its id.
  Future<String> startChat(String username);

  /// Up to [limit] messages of [chat], newest first, older than [before].
  Future<List<MessageView>> messages(
    String chat, {
    int? before,
    int limit = 60,
  });

  /// Sends [text] into [chat].
  Future<void> sendText(String chat, String text);

  /// Marks [chat] as read. People who wrote there learn it only if the
  /// user has written in the chat too.
  Future<void> markRead(String chat);

  /// Accepts a chat request: the people there learn the user's name.
  Future<void> acceptChat(String chat);

  /// Removes [chat] from this device, history and all. Nobody is told.
  Future<void> deleteChat(String chat);
}
