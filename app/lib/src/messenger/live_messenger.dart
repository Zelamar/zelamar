import 'dart:async';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/notifier.dart';
import 'package:zelamar/src/messenger/os_vault.dart';
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
    this.linkPoll = const Duration(seconds: 2),
  }) : notifier = notifier ?? Notifier.forPlatform();

  /// Where the encrypted store lives.
  final String directory;
  final KeyVault vault;
  final ServiceTransport transport;

  /// Tells the user about new messages while the app is not in front.
  final Notifier notifier;

  /// How often both devices look at their link slot while linking.
  final Duration linkPoll;

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
  int _linkGeneration = 0;

  bool _healing = false;
  bool _healAgain = false;

  /// A device just approved: healing waits until it has registered.
  String? _awaitedDevice;

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
  void dispose() {
    _closed = true;
    _retry?.cancel();
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
    _setPhase(MessengerPhase.ready);
    unawaited(notifier.requestPermission());
    await _refreshChats(null);
    unawaited(_runMailbox());
    unawaited(_flush());
  }

  /// Keeps one live connection to the mailbox, reconnecting with backoff.
  Future<void> _runMailbox() async {
    var delay = const Duration(seconds: 1);
    while (!_closed) {
      _setLink(Link.connecting);
      try {
        final socket = await transport.connect(_ready.mailbox());
        _socket = socket;
        _setLink(Link.online);
        delay = const Duration(seconds: 1);
        unawaited(_flush());
        unawaited(_heal());
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
      _socket = null;
      if (_closed) {
        return;
      }
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
        }
      } while (_flushAgain);
      if (lost) {
        // Someone else changed a chat first; whatever ours was for is
        // still missing, and healing finds it again.
        unawaited(_heal());
      }
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
        if (plan.gaps.isNotEmpty) {
          await _refreshChats(null);
          unawaited(_flush());
        }
      } while (_healAgain && !_closed);
    } on Object {
      // The next connection heals again.
    } finally {
      _healing = false;
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
