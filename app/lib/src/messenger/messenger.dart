import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:zelamar/src/calls/call_state.dart';
import 'package:zelamar/src/messenger/outgoing_file.dart';
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

  /// This device was removed from its account: nothing works until it is
  /// erased ([Messenger.eraseThisDevice]).
  unlinked,
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

  /// The account key's fingerprint, once there is an account.
  String? get fingerprint;

  Link get link;

  /// How many Zelamar devices nearby take messages while the service is
  /// out of reach (ADR-0020); 0 while connected.
  int get nearby;

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

  /// Whether this device can link new devices: only the one that created
  /// the account holds its key.
  bool get canLinkDevices;

  /// Joins an existing account as a new device, by the [code] its other
  /// device shows. Once the request is in, [onMatch] gets six digits the
  /// other device shows too. Completes when the other device approved and
  /// this one is ready; [cancelJoin] ends it with `cancelled`.
  Future<void> joinAccount({
    required String code,
    required void Function(String matchCode) onMatch,
  });

  /// Stops [joinAccount] waiting.
  void cancelJoin();

  /// Recovers the account [username] on this device with its recovery
  /// [key] (ADR-0018); completes once this device is ready. Fails with
  /// `wrong_recovery_key` when the name or the key is wrong.
  Future<void> recoverAccount({required String username, required String key});

  /// The recovery key, as shown: only on the device that holds the
  /// account key ([canLinkDevices]), once made.
  String? get recoveryKey;

  /// Whether the service holds the newest recovery kit; `false` while it
  /// waits for a connection.
  bool get recoveryKeySaved;

  /// Makes a new recovery key, replacing any earlier one, and leaves the
  /// sealed kit at the service; returns the key to show.
  Future<String> newRecoveryKey();

  /// Starts linking a new device; returns the code to show.
  Future<String> offerLink();

  /// Waits until a new device answers the code from [offerLink]; `null`
  /// once the offer is cancelled or expired.
  Future<LinkApplicant?> nextApplicant();

  /// Lets the device that answered join the account. It is added to every
  /// chat as soon as it has registered.
  Future<void> approveLink();

  /// Stops offering a link.
  void cancelLink();

  /// The account's devices, this one first.
  Future<List<DeviceView>> devices();

  /// Removes another device from the account: it leaves every chat and
  /// gets nothing more. Only a device that [canLinkDevices] can.
  Future<void> removeDevice(String id);

  /// Removes this device from its account, then erases it.
  Future<void> unlinkThisDevice();

  /// Erases everything this device keeps: its keys, chats and messages.
  Future<void> eraseThisDevice();

  /// Starts a chat with [username]; returns its id.
  Future<String> startChat(String username);

  /// Starts a group called [name] with [usernames]; returns its id.
  Future<String> createGroup(String name, List<String> usernames);

  /// Adds [usernames] to the group [chat].
  Future<void> addMembers(String chat, List<String> usernames);

  /// Renames the group [chat] for everyone in it.
  Future<void> renameChat(String chat, String name);

  /// Leaves the group [chat] and removes it from this device; the others
  /// are told.
  Future<void> leaveChat(String chat);

  /// Everyone else in [chat].
  Future<List<MemberView>> members(String chat);

  /// Whether Zelamar starts in the tray at sign-in; `null` where the
  /// platform has no such thing.
  Future<bool?> startsAtSignIn();

  /// Turns starting at sign-in on or off; returns the new state.
  Future<bool?> setStartsAtSignIn({required bool on});

  /// Up to [limit] messages of [chat], newest first, older than [before].
  Future<List<MessageView>> messages(
    String chat, {
    int? before,
    int limit = 60,
  });

  /// Sends [text] into [chat].
  Future<void> sendText(String chat, String text);

  /// Sends [file] into [chat]: sealed here, uploaded, then announced in a
  /// message (ADR-0016).
  Future<void> sendFile(String chat, OutgoingFile file);

  /// The file in [message] of [chat], fetched and opened; kept in memory
  /// for a while.
  Future<Uint8List> openFile(String chat, String message);

  /// Sends [text] into [chat] as an answer to message [to].
  Future<void> replyText(String chat, String to, String text);

  /// Replaces the text of one of the user's own messages, for everyone.
  Future<void> editText(String chat, String target, String text);

  /// Deletes one of the user's own messages, for everyone.
  Future<void> deleteMessage(String chat, String target);

  /// Sets the user's reaction to a message; `null` removes it.
  Future<void> react(String chat, String target, String? reaction);

  /// The safety code with the one other person in [chat], as twelve groups
  /// of five digits; `null` when the chat has more people or nobody else.
  Future<List<String>?> safetyCode(String chat);

  /// Marks [chat] as read. People who wrote there learn it only if the
  /// user has written in the chat too.
  Future<void> markRead(String chat);

  /// Accepts a chat request: the people there learn the user's name.
  Future<void> acceptChat(String chat);

  /// Removes [chat] from this device, history and all. Nobody is told.
  Future<void> deleteChat(String chat);

  /// The call ringing or going on, or one that just ended (ADR-0019).
  CallState? get call;

  /// Calls the other person in [chat].
  Future<void> startCall(String chat, {bool video = false});

  /// Picks up the call ringing here.
  Future<void> acceptCall();

  /// Turns down the call ringing here.
  Future<void> declineCall();

  /// Ends the call, or stops calling.
  Future<void> hangUp();

  /// Turns the microphone off or on during a call.
  Future<void> setMuted({required bool muted});

  /// Sends a call's sound to the loudspeaker or back to the earpiece.
  Future<void> setSpeaker({required bool on});

  /// The other person's picture during a video call, once it arrives.
  ValueListenable<Widget?> get remoteVideo;

  /// This device's camera during a video call, while it is on.
  ValueListenable<Widget?> get localVideo;

  /// Turns the camera off or on during a video call.
  Future<void> setCamera({required bool on});

  /// Switches between the front and back camera (phones).
  Future<void> switchCamera();
}
