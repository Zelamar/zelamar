import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/live_messenger.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/notifier.dart';
import 'package:zelamar/src/messenger/transport.dart';
import 'package:zelamar/src/rust/frb_generated.dart';

/// Push wake-ups end to end (ADR-0017), on an Android emulator with Google
/// Play services, against a service that holds a real Firebase key.
///
/// Alice's app registers its Firebase token and goes away; Bob writes; the
/// service asks Firebase to wake her. `tool/push_check.sh` runs this test,
/// sends the app to the background when it logs `ZELAMAR_PUSH: away`, and
/// looks for the notification after `ZELAMAR_PUSH: sent`
/// (`.github/workflows/push-check.yml`).
///
/// Skipped without a service and two invites.
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(RustLib.init);

  const service = String.fromEnvironment('ZELAMAR_SERVICE');
  const invites = String.fromEnvironment('ZELAMAR_TEST_INVITES');

  test(
    'a message wakes an app that is away, through Firebase',
    () async {
      final codes = invites.split(',');
      final run = Random().nextInt(1 << 30).toRadixString(36);
      final away = <LiveMessenger>{};

      Future<(LiveMessenger, Directory)> person(
        String name,
        Notifier notifier,
      ) async {
        final directory = await Directory.systemTemp.createTemp('zl-$name-');
        final messenger = LiveMessenger(
          directory: directory.path,
          vault: KeyVault(directory.path, profile: 'push-$name-$run'),
          transport: ServiceTransport(Uri.parse(service)),
          notifier: notifier,
        );
        addTearDown(() {
          if (!away.contains(messenger)) {
            messenger.dispose();
          }
        });
        await messenger.start();
        return (messenger, directory);
      }

      Future<void> until(bool Function() condition, String what) async {
        final deadline = DateTime.now().add(const Duration(seconds: 60));
        while (!condition()) {
          if (DateTime.now().isAfter(deadline)) {
            fail('Timed out waiting until $what');
          }
          await Future<void>.delayed(const Duration(milliseconds: 200));
        }
      }

      final (alice, aliceDirectory) = await person(
        'alice',
        Notifier.forPlatform(),
      );
      final (bob, _) = await person('bob', const SilentNotifier());
      final aliceName = 'push_${run}_a';
      final bobName = 'push_${run}_b';
      await alice.createAccount(username: aliceName, invite: codes[0]);
      await bob.createAccount(username: bobName, invite: codes[1]);

      // Firebase gives Alice's app a token, and her mailbox keeps it.
      expect(await alice.notifier.pushToken(), isNotNull);
      final registered = File(
        '${aliceDirectory.path}${Platform.pathSeparator}push-token',
      );
      await until(registered.existsSync, 'the push token is registered');

      await until(
        () => alice.link == Link.online && bob.link == Link.online,
        'both are online',
      );
      final chat = await alice.startChat(bobName);
      await alice.sendText(chat, 'Hi Bob');
      await until(
        () => bob.chats.any((c) => c.last?.text == 'Hi Bob'),
        'Bob receives the message',
      );

      // Alice's app goes to the background and lets go of its mailbox.
      debugPrint('ZELAMAR_PUSH: away');
      await Future<void>.delayed(const Duration(seconds: 5));
      away.add(alice);
      alice.dispose();
      await Future<void>.delayed(const Duration(seconds: 3));

      await bob.sendText(chat, 'Are you there?');
      debugPrint('ZELAMAR_PUSH: sent');
      // The script looks for the notification meanwhile.
      await Future<void>.delayed(const Duration(seconds: 90));
    },
    skip: invites.isEmpty ? 'needs a service with a Firebase key' : false,
    timeout: const Timeout(Duration(minutes: 5)),
  );
}
