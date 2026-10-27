import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:zelamar/src/calls/call_controller.dart';
import 'package:zelamar/src/calls/call_state.dart';
import 'package:zelamar/src/calls/media_link.dart';
import 'package:zelamar/src/calls/tone_ringer.dart';
import 'package:zelamar/src/calls/webrtc_link.dart';
import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/notifier.dart';
import 'package:zelamar/src/messenger/os_vault.dart';
import 'package:zelamar/src/messenger/outgoing_file.dart';
import 'package:zelamar/src/messenger/transport.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/rust/api/session.dart'
    as core
    show normalizeUsername;

/// How long a link code works (`protocol/linking.md`).
const _linkLifetime = Duration(minutes: 10);

/// What this device calls itself when it asks to join an account.
String _deviceLabel() {
  if (Platform.isAndroid) {
    return 'Android phone';
  }
  if (Platform.isIOS) {
    return 'iPhone';
  }
  if (Platform.isWindows) {
    return 'Windows computer';
  }
  if (Platform.isMacOS) {
    return 'Mac';
  }
  return 'Computer';
}

/// The real messenger: the Rust core's [Session] plus the network.
///
/// Every message the user writes goes into the core's encrypted outbox
/// first, so nothing is lost offline; [_flush] sends the outbox whenever
/// the service is reachable. The mailbox connection delivers incoming
/// envelopes, which the core opens one at a time, in order.
class LiveMessenger extends Messenger {
  new({
    required this.directory,
    required this.vault,
    required this.transport,
    Notifier? notifier,
    MediaLinkFactory? links,
    Ringer? ringer,
    this.linkPoll = const Duration(seconds: 2),
    this.memberHealing = const Duration(minutes: 5),
  }) : notifier = notifier ?? Notifier.forPlatform() {
    _calls =
        CallController(
            signaling: _ChatSignaling(this),
            links: links ?? WebRtcLink.new,
            ringer: ringer ?? ToneRinger(),
          )
          ..onRing = _rang
          ..addListener(notifyListeners)
          ..addListener(_callChanged);
    _callActions = this.notifier.callActions.listen((action) {
      switch (action) {
        case 'accept':
          unawaited(acceptCall());
        case 'decline':
          unawaited(declineCall());
        case 'wake':
          _wake();
      }
    });
  }

  /// Where the encrypted store lives.
  final String directory;
  final KeyVault vault;
  final ServiceTransport transport;

  /// Tells the user about new messages while the app is not in front.
  final Notifier notifier;

  /// How often both devices look at their link slot while linking.
  final Duration linkPoll;

  /// How often healing runs while connected, and looks up other accounts'
  /// devices (ADR-0018).
  final Duration memberHealing;

  /// How long a gap another member should fill may last before this
  /// device fills it instead (ADR-0018).
  static const _gapPatience = Duration(hours: 1);

  Session? _session;
  MessengerPhase _phase = MessengerPhase.starting;
  MessengerFailure? _failure;
  String? _username;
  Link _link = Link.offline;
  List<ChatPreview> _chats = const [];
  final _changes = StreamController<String?>.broadcast();

  WebSocket? _socket;
  bool _closed = false;
  bool _flushing = false;
  bool _flushAgain = false;
  Timer? _retry;
  final _confirmAttempted = <String>{};

  /// Bumped to stop whatever linking loop is waiting.
  int _joinGeneration = 0;

  /// Bumped to stop the mailbox loop when this device goes away.
  int _mailboxGeneration = 0;
  int _linkGeneration = 0;

  bool _healing = false;
  bool _healAgain = false;

  /// A device just approved: healing waits until it has registered.
  String? _awaitedDevice;

  /// When healing last looked up other accounts' devices.
  DateTime? _membersHealed;

  /// Heals every [memberHealing], so a long-running app (in the tray)
  /// still brings a recovered contact back into its chats.
  Timer? _healTimer;

  /// When each gap in another account's devices was first seen, by
  /// `chat/username`.
  final _gapsSeen = <String, DateTime>{};

  bool _savingKit = false;
  bool _kitSaved = true;

  /// This device's calls (ADR-0019).
  late final CallController _calls;

  /// What the platform asks of calls: accept, decline, wake.
  late final StreamSubscription<String> _callActions;

  /// The call's phase the platform was last told about.
  CallPhase? _toldPhase;

  /// Files opened lately, by `chat/message`, oldest first.
  final _files = <String, Uint8List>{};
  static const int _filesKept = 64 * 1024 * 1024;

  @override
  MessengerPhase get phase => _phase;

  @override
  MessengerFailure? get failure => _failure;

  @override
  String? get username => _username;

  @override
  String? get fingerprint => _session?.fingerprint();

  @override
  bool get canLinkDevices => _session?.canLink() ?? false;

  @override
  Link get link => _link;

  @override
  List<ChatPreview> get chats => _chats;

  @override
  Stream<String?> get chatChanged => _changes.stream;

  /// Opens the store if this device has one.
  Future<void> start() async {
    try {
      if (!sessionExists(directory: directory)) {
        _setPhase(MessengerPhase.needsAccount);
        return;
      }
      final key = await vault.masterKey();
      if (key == null) {
        _fail(const MessengerFailure('key_missing'));
        return;
      }
      final session = await Session.open(directory: directory, masterKey: key);
      _session = session;
      _username = session.username();
      if (_username != null && await vault.isRegistered()) {
        await _becomeReady();
      } else {
        _setPhase(MessengerPhase.needsAccount);
      }
    } on SessionError catch (error) {
      _fail(MessengerFailure(error.code, error.message));
    } on SealFailure catch (error) {
      _fail(MessengerFailure('key_missing', error.reason));
    }
  }

  @override
  String? normalizeUsername(String username) =>
      core.normalizeUsername(username: username);

  @override
  Future<void> createAccount({
    required String username,
    required String invite,
  }) => _guard(() async {
    var session = _session;
    if (session == null) {
      final key = newMasterKey();
      // The key is safe before the store exists: a crash in between never
      // leaves a store nobody can open.
      await vault.storeMasterKey(key);
      session = await Session.create(directory: directory, masterKey: key);
      _session = session;
    }
    if (session.username() == null) {
      final claim = await session.claim(username: username, invite: invite);
      final (status, body) = await transport.send(claim);
      _username = await session.claimed(status: status, body: body);
    }
    await _register(session);
  });

  /// Registers this device under its account's name, then opens the chats.
  Future<void> _register(Session session) async {
    final registration = await session.register();
    final (status, body) = await transport.send(registration);
    await session.registered(status: status, body: body);
    await vault.markRegistered();
    await _becomeReady();
  }

  @override
  Future<void> joinAccount({
    required String code,
    required void Function(String matchCode) onMatch,
  }) => _guard(() async {
    final generation = ++_joinGeneration;
    bool stopped() => generation != _joinGeneration || _closed;
    final Joiner joiner;
    try {
      joiner = Joiner(code: code, label: _deviceLabel());
    } on SessionError catch (error) {
      throw MessengerFailure('bad_link_code', error.message);
    }
    try {
      await _discardUnclaimed();
      final (status, body) = await transport.send(joiner.requestCall());
      joiner.requested(status: status, body: body);
      onMatch(joiner.matchCode());
      // Finishing uses the joiner up; the slot is cleared afterwards.
      final close = joiner.closeCall();
      final key = newMasterKey();
      // As for a new account: the key is safe before the store exists.
      await vault.storeMasterKey(key);
      final deadline = DateTime.now().add(_linkLifetime);
      while (_session == null) {
        await Future<void>.delayed(linkPoll);
        if (stopped()) {
          throw const MessengerFailure('cancelled');
        }
        if (DateTime.now().isAfter(deadline)) {
          throw const MessengerFailure('unknown_link');
        }
        final (int, Uint8List) answer;
        try {
          answer = await transport.send(joiner.checkCall());
        } on MessengerFailure catch (failure) {
          if (failure.code == 'offline') {
            continue;
          }
          rethrow;
        }
        final session = await joiner.finish(
          status: answer.$1,
          body: answer.$2,
          directory: directory,
          masterKey: key,
        );
        if (session != null) {
          _session = session;
          _username = session.username();
          unawaited(_sendQuietly(close));
        }
      }
    } finally {
      joiner.dispose();
    }
    await _register(_session!);
  });

  @override
  void cancelJoin() => _joinGeneration++;

  @override
  Future<void> recoverAccount({
    required String username,
    required String key,
  }) => _guard(() async {
    final Restorer restorer;
    try {
      restorer = Restorer(username: username, key: key);
    } on SessionError catch (error) {
      throw MessengerFailure('bad_recovery_key', error.message);
    }
    try {
      await _discardUnclaimed();
      final (status, body) = await transport.send(restorer.fetchCall());
      final masterKey = newMasterKey();
      // As for a new account: the key is safe before the store exists.
      await vault.storeMasterKey(masterKey);
      final session = await restorer.finish(
        status: status,
        body: body,
        directory: directory,
        masterKey: masterKey,
      );
      _session = session;
      _username = session.username();
    } finally {
      restorer.dispose();
    }
    await _register(_session!);
  });

  @override
  String? get recoveryKey => _session?.recoveryKey();

  @override
  bool get recoveryKeySaved => _kitSaved;

  @override
  Future<String> newRecoveryKey() => _guard(() async {
    final key = await _ready.newRecoveryKey();
    _kitSaved = false;
    notifyListeners();
    await _saveKit();
    return key;
  });

  /// Leaves the recovery kit at the service whenever what it holds changed:
  /// a new key, contact or chat (ADR-0018).
  Future<void> _saveKit() async {
    if (_savingKit || _closed) {
      return;
    }
    _savingKit = true;
    final saved = _kitSaved;
    try {
      final session = _ready;
      final call = await session.recoveryCall();
      if (call != null) {
        final (status, body) = await transport.send(call);
        await session.recoverySaved(status: status, body: body);
      }
      _kitSaved = true;
    } on Object {
      // The next change, or the next connection, tries again.
      _kitSaved = false;
    } finally {
      _savingKit = false;
      if (saved != _kitSaved) {
        notifyListeners();
      }
    }
  }

  /// Removes an account that never got a name, so this device can join
  /// another one instead. Nothing in it can be lost.
  Future<void> _discardUnclaimed() async {
    final session = _session;
    if (session == null) {
      return;
    }
    if (session.username() != null) {
      throw const MessengerFailure('already_initialized');
    }
    _session = null;
    session.dispose();
    removeStore(directory: directory);
  }

  @override
  Future<String> offerLink() => _guard(() async {
    _linkGeneration++;
    return await _ready.offerLink();
  });

  @override
  Future<LinkApplicant?> nextApplicant() => _guard(() async {
    final generation = _linkGeneration;
    final deadline = DateTime.now().add(_linkLifetime);
    while (DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(linkPoll);
      if (generation != _linkGeneration || _closed) {
        return null;
      }
      try {
        final session = _ready;
        final (status, body) = await transport.send(session.linkCheckCall());
        final applicant = await session.linkApplicant(
          status: status,
          body: body,
        );
        if (applicant != null) {
          return applicant;
        }
      } on MessengerFailure catch (failure) {
        if (failure.code != 'offline') {
          rethrow;
        }
      }
    }
    return null;
  });

  @override
  Future<void> approveLink() => _guard(() async {
    final session = _ready;
    final (status, body) = await transport.send(await session.approveLink());
    final device = await session.linkApproved(status: status, body: body);
    _linkGeneration++;
    unawaited(_heal(waitFor: device));
  });

  @override
  void cancelLink() {
    _linkGeneration++;
    _session?.cancelLink();
  }

  @override
  Future<List<DeviceView>> devices() => _guard(() async {
    final session = _ready;
    final (status, body) = await transport.send(session.devicesCall());
    return await session.deviceViews(status: status, body: body);
  });

  @override
  Future<void> removeDevice(String id) => _guard(() async {
    final session = _ready;
    final (status, body) = await transport.send(
      await session.removeDeviceCall(device: id),
    );
    session.deviceRemoved(status: status, body: body);
    unawaited(_heal());
  });

  @override
  Future<void> unlinkThisDevice() => _guard(() async {
    final session = _ready;
    final me = (await devices()).firstWhere((device) => device.thisDevice);
    final (status, body) = await transport.send(
      await session.removeDeviceCall(device: me.id),
    );
    session.deviceRemoved(status: status, body: body);
    await eraseThisDevice();
  });

  /// The push token last left with the mailbox, kept next to the store.
  File get _pushTokenFile =>
      File('$directory${Platform.pathSeparator}push-token');

  /// Leaves this device's push token with its mailbox, once per token.
  Future<void> _registerPush() async {
    try {
      final token = await notifier.pushToken();
      final file = _pushTokenFile;
      if (token == null ||
          (file.existsSync() && await file.readAsString() == token)) {
        return;
      }
      final (status, body) = await transport.send(
        _ready.pushCall(token: token),
      );
      pushRegistered(status: status, body: body);
      await file.writeAsString(token, flush: true);
    } on Object {
      // The next start tries again; messages still arrive while open.
    }
  }

  /// Takes the push token back, so an erased app is never woken.
  Future<void> _forgetPush() async {
    final file = _pushTokenFile;
    if (!file.existsSync()) {
      return;
    }
    try {
      final session = _session;
      if (session != null) {
        final (status, body) = await transport.send(session.pushCall());
        pushRegistered(status: status, body: body);
      }
    } on Object {
      // The mailbox is orphaned anyway once the device is gone.
    }
    await file.delete();
  }

  @override
  Future<void> eraseThisDevice() => _guard(() async {
    await _forgetPush();
    _offline();
    final session = _session;
    _session = null;
    session?.dispose();
    removeStore(directory: directory);
    await vault.forget();
    _username = null;
    _chats = const [];
    _setPhase(MessengerPhase.needsAccount);
  });

  /// Stops talking to the service: no mailbox, no retries.
  void _offline() {
    _mailboxGeneration++;
    _retry?.cancel();
    unawaited(_socket?.close());
    _socket = null;
    _setLink(Link.offline);
  }

  @override
  Future<String> startChat(String username) => _guard(() async {
    final session = _ready;
    final (status, body) = await transport.send(lookupCall(username: username));
    final chat = await session.startChat(
      username: username,
      status: status,
      body: body,
      own: await _ownDevices(),
    );
    await _refreshChats(chat);
    unawaited(_flush());
    return chat;
  });

  @override
  Future<String> createGroup(String name, List<String> usernames) =>
      _guard(() async {
        final session = _ready;
        final invitees = await _invitees(usernames);
        final chat = await session.createGroup(
          name: name,
          invitees: invitees,
          own: await _ownDevices(),
        );
        await _refreshChats(chat);
        unawaited(_flush());
        return chat;
      });

  @override
  Future<void> addMembers(String chat, List<String> usernames) =>
      _guard(() async {
        final invitees = await _invitees(usernames);
        await _ready.addMembers(chat: chat, invitees: invitees);
        await _refreshChats(chat);
        unawaited(_flush());
      });

  @override
  Future<void> renameChat(String chat, String name) => _changeChat(
    chat,
    (session) => session.renameChat(chat: chat, name: name),
  );

  @override
  Future<void> leaveChat(String chat) => _guard(() async {
    await _ready.leaveChat(chat: chat);
    await _refreshChats(null);
    unawaited(_flush());
  });

  @override
  Future<List<MemberView>> members(String chat) =>
      _guard(() => _ready.members(chat: chat));

  @override
  Future<bool?> startsAtSignIn() => notifier.startsAtSignIn();

  @override
  Future<bool?> setStartsAtSignIn({required bool on}) =>
      notifier.setStartsAtSignIn(on: on);

  /// Looks each of [usernames] up in the directory.
  Future<List<Invitee>> _invitees(List<String> usernames) async {
    final invitees = <Invitee>[];
    for (final username in usernames) {
      final (status, body) = await transport.send(
        lookupCall(username: username),
      );
      invitees.add(
        Invitee(
          username: username,
          answer: HttpAnswer(status: status, body: body),
        ),
      );
    }
    return invitees;
  }

  /// Takes the devices of people who left out of the chats where this
  /// device is the one to do it (ADR-0015).
  Future<void> _seeOff() async {
    try {
      if (await _ready.removeDeparted() > 0) {
        await _refreshChats(null);
        unawaited(_flush());
      }
    } on Object {
      // The next message or connection tries again.
    }
  }

  /// This account's own devices with a key package each, so a new chat
  /// holds them from the start; `null` when the service cannot say.
  Future<HttpAnswer?> _ownDevices() async {
    final name = _username;
    if (name == null) {
      return null;
    }
    try {
      final (status, body) = await transport.send(lookupCall(username: name));
      return status == 200 ? HttpAnswer(status: status, body: body) : null;
    } on MessengerFailure {
      return null;
    }
  }

  @override
  Future<List<MessageView>> messages(
    String chat, {
    int? before,
    int limit = 60,
  }) => _guard(() => _ready.messages(chat: chat, before: before, limit: limit));

  @override
  Future<void> sendText(String chat, String text) => _guard(() async {
    await _ready.sendText(chat: chat, text: text);
    await _refreshChats(chat);
    unawaited(_flush());
  });

  @override
  Future<void> sendFile(String chat, OutgoingFile file) => _guard(() async {
    final session = _ready;
    final prepared = await session.prepareFile(bytes: file.bytes);
    for (final call in prepared.calls) {
      final (status, body) = await transport.send(call);
      chunkUploaded(status: status, body: body);
    }
    await session.sendFile(
      chat: chat,
      blob: prepared.blob,
      name: file.name,
      mime: file.mime,
      width: file.width,
      height: file.height,
      thumb: file.thumb,
      durationMs: file.durationMs,
    );
    await _refreshChats(chat);
    unawaited(_flush());
  });

  @override
  Future<Uint8List> openFile(String chat, String message) => _guard(() async {
    final key = '$chat/$message';
    final kept = _files.remove(key);
    if (kept != null) {
      _files[key] = kept;
      return kept;
    }
    final session = _ready;
    final calls = await session.downloadCalls(chat: chat, message: message);
    final answers = <HttpAnswer>[];
    for (final call in calls) {
      final (status, body) = await transport.send(call);
      answers.add(HttpAnswer(status: status, body: body));
    }
    final bytes = await session.openFile(
      chat: chat,
      message: message,
      answers: answers,
    );
    _files[key] = bytes;
    var total = _files.values.fold<int>(0, (sum, file) => sum + file.length);
    while (total > _filesKept && _files.length > 1) {
      total -= _files.remove(_files.keys.first)!.length;
    }
    return bytes;
  });

  @override
  Future<void> markRead(String chat) => _guard(() async {
    final covered = await _ready.markRead(chat: chat);
    if (covered > 0) {
      await _refreshChats(null);
      unawaited(_flush());
    }
  });

  @override
  Future<void> replyText(String chat, String to, String text) => _changeChat(
    chat,
    (session) => session.replyText(chat: chat, to: to, text: text),
  );

  @override
  Future<void> editText(String chat, String target, String text) => _changeChat(
    chat,
    (session) => session.editText(chat: chat, target: target, text: text),
  );

  @override
  Future<void> deleteMessage(String chat, String target) => _changeChat(
    chat,
    (session) => session.deleteMessage(chat: chat, target: target),
  );

  @override
  Future<void> react(String chat, String target, String? reaction) =>
      _changeChat(
        chat,
        (session) =>
            session.react(chat: chat, target: target, reaction: reaction),
      );

  @override
  Future<List<String>?> safetyCode(String chat) =>
      _guard(() => _ready.safetyCode(chat: chat));

  /// Shows what was just queued in [chat], and sends it.
  Future<void> _sent(String chat) async {
    await _refreshChats(chat);
    unawaited(_flush());
  }

  /// Runs a change to [chat] in the core, then shows it and sends it.
  Future<void> _changeChat(
    String chat,
    Future<void> Function(Session session) change,
  ) => _guard(() async {
    await change(_ready);
    await _refreshChats(chat);
    unawaited(_flush());
  });

  @override
  Future<void> acceptChat(String chat) => _guard(() async {
    await _ready.acceptChat(chat: chat);
    await _refreshChats(chat);
    unawaited(_flush());
    // The account has spoken there now: its other devices belong in it.
    unawaited(_heal());
  });

  @override
  Future<void> deleteChat(String chat) => _guard(() async {
    await _ready.deleteChat(chat: chat);
    await _refreshChats(null);
  });

  @override
  CallState? get call => _calls.state;

  @override
  Future<void> startCall(String chat, {bool video = false}) => _guard(() {
    final preview = _chats.where((preview) => preview.id == chat).firstOrNull;
    if (preview == null || preview.group) {
      throw const MessengerFailure('not_one_to_one');
    }
    _ready;
    return _calls.start(chat, peer: preview.title, video: video);
  });

  @override
  Future<void> acceptCall() => _calls.accept();

  @override
  Future<void> declineCall() => _calls.decline();

  @override
  Future<void> hangUp() => _calls.hangUp();

  @override
  Future<void> setMuted({required bool muted}) => _calls.setMuted(muted: muted);

  @override
  Future<void> setSpeaker({required bool on}) => _calls.setSpeaker(on: on);

  @override
  ValueListenable<Widget?> get remoteVideo => _calls.remoteVideo;

  @override
  ValueListenable<Widget?> get localVideo => _calls.localVideo;

  @override
  Future<void> setCamera({required bool on}) => _calls.setCamera(on: on);

  @override
  Future<void> switchCamera() => _calls.switchCamera();

  /// A call rings here: if Zelamar is out of sight, the platform rings
  /// like a phone (Android: the ringtone, over the lock screen) or brings
  /// the window to the front (Windows).
  Future<void> _rang(CallState call) async {
    if (await notifier.inFront()) {
      return;
    }
    if (Platform.isAndroid) {
      // The phone's own ringtone rings instead.
      _calls.ringer.stop();
    }
    await notifier.ring(title: call.peer, video: call.video);
  }

  /// Tells the platform when a call stops ringing, and when one starts or
  /// stops going on.
  void _callChanged() {
    final call = _calls.state;
    final phase = call?.phase;
    final was = _toldPhase;
    if (phase == was) {
      return;
    }
    _toldPhase = phase;
    bool talking(CallPhase? phase) =>
        phase == CallPhase.connecting || phase == CallPhase.active;
    if (was == CallPhase.incoming) {
      unawaited(notifier.stopRinging());
    }
    if (call != null && talking(phase) && !talking(was)) {
      unawaited(
        notifier.callOngoing(on: true, title: call.peer, video: call.video),
      );
    } else if (talking(was) && !talking(phase) || phase == null) {
      unawaited(notifier.callOngoing(on: false));
    }
  }

  /// A call wake-up arrived while Zelamar runs: fetch the mail now, even if
  /// the connection is waiting to try again.
  void _wake() {
    if (_phase != MessengerPhase.ready || _link == Link.online) {
      return;
    }
    unawaited(_socket?.close());
    unawaited(_runMailbox());
  }

  @override
  void dispose() {
    _closed = true;
    unawaited(_callActions.cancel());
    _calls.dispose();
    _retry?.cancel();
    _healTimer?.cancel();
    unawaited(_socket?.close());
    unawaited(_changes.close());
    transport.close();
    // Releases the core's device, which closes the encrypted store.
    _session?.dispose();
    _session = null;
    super.dispose();
  }

  Session get _ready {
    final session = _session;
    if (session == null || _phase != MessengerPhase.ready) {
      throw const MessengerFailure('not_ready');
    }
    return session;
  }

  Future<void> _becomeReady() async {
    try {
      await _session?.setOwnLabel(label: _deviceLabel());
    } on SessionError {
      // Only a name for the devices screen.
    }
    _setPhase(MessengerPhase.ready);
    unawaited(notifier.requestPermission());
    unawaited(_registerPush());
    await _refreshChats(null);
    unawaited(_runMailbox());
    unawaited(_flush());
    unawaited(_saveKit());
    _healTimer ??= Timer.periodic(memberHealing, (_) {
      if (_link == Link.online) {
        unawaited(_heal());
      }
    });
  }

  /// Keeps one live connection to the mailbox, reconnecting with backoff.
  Future<void> _runMailbox() async {
    final generation = ++_mailboxGeneration;
    bool stopped() => _closed || generation != _mailboxGeneration;
    var delay = const Duration(seconds: 1);
    while (!stopped()) {
      _setLink(Link.connecting);
      try {
        final socket = await transport.connect(_ready.mailbox());
        if (stopped()) {
          await socket.close();
          return;
        }
        _socket = socket;
        _setLink(Link.online);
        delay = const Duration(seconds: 1);
        unawaited(_flush());
        unawaited(_heal());
        unawaited(_seeOff());
        // `await for` opens envelopes strictly one after another, in the
        // order the mailbox sends them.
        await for (final data in socket) {
          if (data is List<int>) {
            await _open(socket, data);
          }
        }
      } on Object {
        // Reconnect below.
      }
      if (stopped()) {
        return;
      }
      _socket = null;
      _setLink(Link.offline);
      await Future<void>.delayed(delay);
      delay = Duration(seconds: min(delay.inSeconds * 2, 30));
    }
  }

  Future<void> _open(WebSocket socket, List<int> data) async {
    final event = await _ready.openEnvelope(bytes: data);
    final ack = event.ack;
    if (ack != null) {
      socket.add(ack);
    }
    if (event.chat != null) {
      await _refreshChats(event.chat);
      unawaited(_confirmNames());
      // Opening it may have queued a receipt: "delivered", and for a call,
      // "it rings here".
      unawaited(_flush());
    }
    if (event.left) {
      // Perhaps the account removed this device; healing finds out.
      unawaited(_heal());
    } else if (event.chat != null) {
      // Someone may have left; their devices go.
      unawaited(_seeOff());
    }
    final call = event.call;
    if (call != null) {
      _calls.handle(call);
    }
    final notice = event.notice;
    if (notice != null && !await notifier.inFront()) {
      await notifier.show(title: notice.title, body: notice.body);
    }
  }

  /// Sends everything in the outbox, oldest first. Retries later when the
  /// service cannot be reached.
  Future<void> _flush() async {
    if (_flushing) {
      _flushAgain = true;
      return;
    }
    _flushing = true;
    var lost = false;
    try {
      do {
        _flushAgain = false;
        final pending = await _ready.pendingSends();
        for (final item in pending) {
          final (status, body) = await transport.send(item.call);
          lost |= await _ready.sent(id: item.id, status: status, body: body);
        }
        if (pending.isNotEmpty) {
          await _refreshChats(null);
          // Settling may queue more, such as a group's name for people
          // just added.
          _flushAgain = true;
        }
      } while (_flushAgain);
      if (lost) {
        // Someone else changed a chat first; whatever ours was for is
        // still missing, and healing finds it again.
        unawaited(_heal());
      }
      // A new chat or contact belongs in the recovery kit.
      unawaited(_saveKit());
    } on Object {
      _retry?.cancel();
      _retry = Timer(const Duration(seconds: 5), () => unawaited(_flush()));
    } finally {
      _flushing = false;
    }
  }

  /// Brings this account's devices into every chat that lacks them
  /// (ADR-0014). With [waitFor], first waits until that device has
  /// registered.
  Future<void> _heal({String? waitFor}) async {
    _awaitedDevice = waitFor ?? _awaitedDevice;
    if (_healing) {
      _healAgain = true;
      return;
    }
    _healing = true;
    final started = DateTime.now();
    try {
      do {
        _healAgain = false;
        final session = _ready;
        final (status, body) = await transport.send(session.devicesCall());
        final plan = await session.healPlan(status: status, body: body);
        if (!plan.thisDeviceListed) {
          // The account removed this device: it may not read or write
          // anything more, only be erased.
          _offline();
          _setPhase(MessengerPhase.unlinked);
          return;
        }
        for (final gap in plan.stale) {
          await session.removeOwnDevices(chat: gap.chat, devices: gap.devices);
        }
        final awaited = _awaitedDevice;
        if (awaited != null && !plan.devices.contains(awaited)) {
          if (DateTime.now().difference(started) > _linkLifetime) {
            _awaitedDevice = null;
            return;
          }
          await Future<void>.delayed(linkPoll);
          _healAgain = true;
          continue;
        }
        _awaitedDevice = null;
        for (final gap in plan.gaps) {
          final answers = <HttpAnswer>[];
          for (final device in gap.devices) {
            final (status, body) = await transport.send(
              session.deviceLookupCall(device: device),
            );
            answers.add(HttpAnswer(status: status, body: body));
          }
          await session.addOwnDevices(chat: gap.chat, answers: answers);
        }
        if (plan.gaps.isNotEmpty || plan.stale.isNotEmpty) {
          await _refreshChats(null);
          unawaited(_flush());
        }
      } while (_healAgain && !_closed);
      await _healMembers();
    } on Object {
      // The next connection heals again.
    } finally {
      _healing = false;
    }
  }

  /// Brings other accounts' devices into the chats where none of theirs is
  /// left, as after a recovery (ADR-0018). The member with the lowest leaf
  /// does it at once; any other after [_gapPatience]. Looks at most every
  /// [memberHealing].
  Future<void> _healMembers() async {
    final now = DateTime.now();
    final last = _membersHealed;
    if (last != null && now.difference(last) < memberHealing) {
      return;
    }
    _membersHealed = now;
    final session = _ready;
    final names = await session.directoryNames();
    final answers = <DirectoryAnswer>[];
    for (var at = 0; at < names.length; at += 64) {
      final batch = names.sublist(at, min(at + 64, names.length));
      final (status, body) = await transport.send(
        devicesOfCall(usernames: batch),
      );
      answers.add(
        DirectoryAnswer(
          usernames: batch,
          answer: HttpAnswer(status: status, body: body),
        ),
      );
    }
    final gaps = await session.memberGaps(answers: answers);
    final open = <String>{};
    var changed = false;
    for (final gap in gaps) {
      final key = '${gap.chat}/${gap.username}';
      open.add(key);
      final seen = _gapsSeen.putIfAbsent(key, () => now);
      if (!gap.first && now.difference(seen) < _gapPatience) {
        continue;
      }
      final lookups = <HttpAnswer>[];
      for (final device in gap.devices) {
        final (status, body) = await transport.send(
          memberDeviceCall(username: gap.username, device: device),
        );
        lookups.add(HttpAnswer(status: status, body: body));
      }
      await session.addMemberDevices(
        chat: gap.chat,
        username: gap.username,
        answers: lookups,
      );
      changed = true;
    }
    _gapsSeen.removeWhere((key, _) => !open.contains(key));
    if (changed) {
      await _refreshChats(null);
      unawaited(_flush());
    }
  }

  /// Sends [call] and ignores the answer and any failure.
  Future<void> _sendQuietly(HttpCall call) async {
    try {
      await transport.send(call);
    } on Object {
      // Nothing depends on it.
    }
  }

  /// Asks the directory about names people claim in chats.
  Future<void> _confirmNames() async {
    try {
      final names = await _ready.namesToConfirm();
      for (final name in names.where(_confirmAttempted.add)) {
        final (status, body) = await transport.send(
          keyLookupCall(username: name),
        );
        await _ready.confirmName(username: name, status: status, body: body);
      }
      if (names.isNotEmpty) {
        await _refreshChats(null);
      }
    } on Object {
      // Names stay unconfirmed; the screens say so.
    }
  }

  Future<void> _refreshChats(String? changed) async {
    final session = _session;
    if (session == null || _closed) {
      return;
    }
    _chats = await session.chats();
    _changes.add(changed);
    notifyListeners();
  }

  /// Runs [action], turning every failure into a [MessengerFailure].
  Future<T> _guard<T>(Future<T> Function() action) async {
    try {
      return await action();
    } on SessionError catch (error) {
      throw MessengerFailure(error.code, error.message);
    }
  }

  void _setPhase(MessengerPhase phase) {
    _phase = phase;
    notifyListeners();
  }

  void _setLink(Link link) {
    if (_link != link && !_closed) {
      _link = link;
      notifyListeners();
    }
  }

  void _fail(MessengerFailure failure) {
    _failure = failure;
    _setPhase(MessengerPhase.failed);
  }
}

/// A call's messages go through its chat, like any message: into the
/// outbox, then out at once.
class _ChatSignaling implements CallSignaling {
  const new(this._messenger);

  final LiveMessenger _messenger;

  @override
  Future<String> offer(
    String chat, {
    required bool video,
    required String sdp,
  }) => _messenger._guard(() async {
    final id = await _messenger._ready.startCall(
      chat: chat,
      video: video,
      sdp: sdp,
    );
    await _messenger._sent(chat);
    return id;
  });

  @override
  Future<void> answer(String chat, String call, String sdp) =>
      _messenger._guard(() async {
        await _messenger._ready.answerCall(chat: chat, call: call, sdp: sdp);
        await _messenger._sent(chat);
      });

  @override
  Future<void> end(
    String chat,
    String call,
    CallEnding ending, {
    int? durationMs,
  }) => _messenger._guard(() async {
    await _messenger._ready.endCall(
      chat: chat,
      call: call,
      ending: ending,
      durationMs: durationMs,
    );
    await _messenger._sent(chat);
  });

  @override
  Future<List<IceServerView>> servers() => _messenger._guard(() async {
    final (status, body) = await _messenger.transport.send(
      _messenger._ready.iceCall(),
    );
    return await iceServers(status: status, body: body);
  });
}
