import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/live_messenger.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/notifier.dart';
import 'package:zelamar/src/messenger/transport.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/rust/frb_generated.dart';

/// Drift D1 for real (ADR-0020): two app instances talk through a running
/// service, then lose it. Within seconds they find each other on the local
/// network, and a message goes straight from one to the other. When the
/// service is back, its copy shows nothing twice.
///
/// Needs a service and two fresh invite codes:
///
///     cd server && npx wrangler dev --port 8787
///     flutter test integration_test/drift_test.dart -d windows \
///       --dart-define=ZELAMAR_SERVICE=http://127.0.0.1:8787 \
///       --dart-define=ZELAMAR_TEST_INVITES=<code>,<code>
///
/// Skipped when no invites are given.
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(RustLib.init);

  const service = String.fromEnvironment(
    'ZELAMAR_SERVICE',
    defaultValue: 'http://127.0.0.1:8787',
  );
  const invites = String.fromEnvironment('ZELAMAR_TEST_INVITES');
  final codes = invites.split(',');
  final run = Random().nextInt(1 << 30).toRadixString(36);

  Future<(LiveMessenger, _Cable)> person(String name) async {
    final directory = await Directory.systemTemp.createTemp('zl-drift-$name-');
    addTearDown(() async {
      for (var attempt = 0; ; attempt++) {
        try {
          await directory.delete(recursive: true);
          return;
        } on FileSystemException {
          if (attempt == 50) rethrow;
          await Future<void>.delayed(const Duration(milliseconds: 100));
        }
      }
    });
    final cable = _Cable(Uri.parse(service));
    final messenger = LiveMessenger(
      directory: directory.path,
      vault: KeyVault(directory.path, profile: 'drift-$name-$run'),
      transport: cable,
      notifier: const SilentNotifier(),
      nearbyAfter: const Duration(seconds: 1),
    );
    addTearDown(messenger.dispose);
    await messenger.start();
    return (messenger, cable);
  }

  Future<void> until(
    bool Function() condition,
    String what, {
    Duration within = const Duration(seconds: 30),
  }) async {
    final deadline = DateTime.now().add(within);
    while (!condition()) {
      if (DateTime.now().isAfter(deadline)) {
        fail('Timed out waiting until $what');
      }
      await Future<void>.delayed(const Duration(milliseconds: 100));
    }
  }

  test(
    'without the internet, a message goes to the person nearby',
    () async {
      final (alice, alicesCable) = await person('alice');
      final (bob, bobsCable) = await person('bob');
      final bobName = 'qd_${run}_b';
      await alice.createAccount(username: 'qd_${run}_a', invite: codes[0]);
      await bob.createAccount(username: bobName, invite: codes[1]);
      await until(
        () => alice.link == Link.online && bob.link == Link.online,
        'both are online',
      );
      final chat = await alice.startChat(bobName);
      await alice.sendText(chat, 'Hi Bob');
      await until(
        () => bob.chats.isNotEmpty && bob.chats.single.last?.text == 'Hi Bob',
        'Bob gets the first message',
      );
      await bob.sendText(chat, 'Hi Alice');
      await until(
        () => alice.chats.single.last?.text == 'Hi Alice',
        'Alice gets the answer',
      );
      expect(alice.nearby, 0, reason: 'with the service, nothing nearby');

      // The internet goes away for both.
      alicesCable.cut = true;
      bobsCable.cut = true;
      await until(
        () => alice.nearby > 0 && bob.nearby > 0,
        'they find each other nearby',
      );

      await alice.sendText(chat, 'The internet is down, are you there?');
      await until(
        () =>
            bob.chats.single.last?.text ==
            'The internet is down, are you there?',
        'Bob gets it from Alice directly',
      );

      // The internet is back: the service brings its copy too.
      alicesCable.cut = false;
      bobsCable.cut = false;
      await until(
        () => alice.link == Link.online && bob.link == Link.online,
        'both are online again',
      );
      await until(() => alice.nearby == 0, 'nothing nearby with the service');
      await Future<void>.delayed(const Duration(seconds: 3));
      final texts = (await bob.messages(chat)).map((m) => m.text).toList();
      expect(
        texts.where((t) => t == 'The internet is down, are you there?'),
        hasLength(1),
        reason: 'shown once',
      );
    },
    skip: invites.isEmpty ? 'needs a service and two invites' : null,
    timeout: const Timeout(Duration(minutes: 3)),
  );
}

/// The way to the service, which a test can cut: requests fail as offline
/// and live connections close, as when the internet goes away.
class _Cable extends ServiceTransport {
  new(super.base);

  bool _cut = false;

  bool get cut => _cut;
  final _sockets = <WebSocket>[];

  set cut(bool value) {
    _cut = value;
    if (value) {
      for (final socket in _sockets) {
        socket.close().ignore();
      }
      _sockets.clear();
    }
  }

  @override
  Future<(int, Uint8List)> send(HttpCall call) {
    if (_cut) {
      throw const MessengerFailure('offline', 'cut');
    }
    return super.send(call);
  }

  @override
  Future<WebSocket> connect(MailboxLink link) async {
    if (_cut) {
      throw const SocketException('cut');
    }
    final socket = await super.connect(link);
    _sockets.add(socket);
    return socket;
  }
}
