import 'dart:io';
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/live_messenger.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/notifier.dart';
import 'package:zelamar/src/messenger/transport.dart';
import 'package:zelamar/src/rust/frb_generated.dart';

/// The whole stack, for real: two app instances, each with its own Rust
/// core, encrypted store and secure key, meet through a running delivery
/// service over HTTP and WebSocket.
///
/// Needs a service and two fresh invite codes:
///
///     cd server && npx wrangler dev --port 8787
///     flutter test integration_test/live_test.dart -d windows \
///       --dart-define=ZELAMAR_SERVICE=http://127.0.0.1:8787 \
///       --dart-define=ZELAMAR_TEST_INVITES=<code>,<code>
///
/// Skipped when no invites are given (as in CI, which has no service yet).
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(RustLib.init);

  const service = String.fromEnvironment(
    'ZELAMAR_SERVICE',
    defaultValue: 'http://127.0.0.1:8787',
  );
  const invites = String.fromEnvironment('ZELAMAR_TEST_INVITES');

  test(
    'two people meet by username and talk through the service',
    () async {
      final codes = invites.split(',');
      final run = Random().nextInt(1 << 30).toRadixString(36);

      Future<LiveMessenger> person(String name) async {
        final directory = await Directory.systemTemp.createTemp('zl-$name-');
        addTearDown(() async {
          // The store closes once the core lets go of the last call in
          // flight; give it a moment.
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
        final messenger = LiveMessenger(
          directory: directory.path,
          vault: KeyVault(directory.path, profile: 'live-$name-$run'),
          transport: ServiceTransport(Uri.parse(service)),
          notifier: const SilentNotifier(),
        );
        addTearDown(messenger.dispose);
        await messenger.start();
        expect(messenger.phase, MessengerPhase.needsAccount);
        return messenger;
      }

      Future<void> until(bool Function() condition, String what) async {
        final deadline = DateTime.now().add(const Duration(seconds: 30));
        while (!condition()) {
          if (DateTime.now().isAfter(deadline)) {
            fail('Timed out waiting until $what');
          }
          await Future<void>.delayed(const Duration(milliseconds: 100));
        }
      }

      final alice = await person('alice');
      final bob = await person('bob');
      final aliceName = 'qa_${run}_a';
      final bobName = 'qa_${run}_b';
      await alice.createAccount(username: aliceName, invite: codes[0]);
      await bob.createAccount(username: bobName, invite: codes[1]);
      expect(alice.phase, MessengerPhase.ready);
      await until(
        () => alice.link == Link.online && bob.link == Link.online,
        'both are online',
      );

      final chat = await alice.startChat(bobName);
      await alice.sendText(chat, 'Hi Bob, it is Alice.');

      await until(
        () => bob.chats.any((c) => c.last?.text == 'Hi Bob, it is Alice.'),
        'Bob receives the message',
      );
      final bobsChat = bob.chats.single;
      expect(bobsChat.id, chat);
      await until(
        () => bob.chats.single.verified,
        'Bob confirms Alice’s name with the directory',
      );
      expect(bob.chats.single.title, '@$aliceName');

      await bob.sendText(chat, 'Hello Alice!');
      await until(
        () => alice.chats.single.last?.text == 'Hello Alice!',
        'Alice receives the reply',
      );
      final mine = await alice.messages(chat);
      expect(mine.map((m) => m.text), ['Hello Alice!', 'Hi Bob, it is Alice.']);
      expect(mine.last.outgoing, isTrue);
    },
    skip: invites.isEmpty ? 'needs a running service and invites' : false,
    timeout: const Timeout(Duration(minutes: 3)),
  );
}
