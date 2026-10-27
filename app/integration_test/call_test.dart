import 'dart:io';
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:zelamar/src/calls/call_state.dart';
import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/live_messenger.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/notifier.dart';
import 'package:zelamar/src/messenger/transport.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/rust/frb_generated.dart';

/// A real call (ADR-0019): two app instances, each with its own Rust core
/// and WebRTC connection, call each other through a running delivery
/// service. The offer and answer travel through the encrypted chat; the
/// media goes straight from one to the other.
///
/// Needs a service and two fresh invite codes:
///
///     cd server && npx wrangler dev --port 8787
///     flutter test integration_test/call_test.dart -d windows \
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

  Future<LiveMessenger> person(String name) async {
    final directory = await Directory.systemTemp.createTemp('zl-call-$name-');
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
    final messenger = LiveMessenger(
      directory: directory.path,
      vault: KeyVault(directory.path, profile: 'call-$name-$run'),
      transport: ServiceTransport(Uri.parse(service)),
      notifier: const SilentNotifier(),
    );
    addTearDown(messenger.dispose);
    await messenger.start();
    return messenger;
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

  Future<CallView?> lastCall(LiveMessenger messenger, String chat) async =>
      (await messenger.messages(chat, limit: 1)).firstOrNull?.call;

  test(
    'two people call each other: it rings, they talk, one hangs up',
    () async {
      final alice = await person('alice');
      final bob = await person('bob');
      final bobName = 'qc_${run}_b';
      await alice.createAccount(username: 'qc_${run}_a', invite: codes[0]);
      await bob.createAccount(username: bobName, invite: codes[1]);
      await until(
        () => alice.link == Link.online && bob.link == Link.online,
        'both are online',
      );
      final chat = await alice.startChat(bobName);
      await alice.sendText(chat, 'Can I call you?');
      await until(() => bob.chats.isNotEmpty, 'Bob gets the chat');
      // Calls ring only in chats the user has accepted.
      await bob.sendText(chat, 'Sure!');
      await bob.markRead(chat);
      await until(
        () => alice.chats.single.last?.text == 'Sure!',
        'Alice gets the answer',
      );

      await alice.startCall(chat);
      await until(
        () => bob.call?.phase == CallPhase.incoming,
        'it rings on Bob’s side',
      );
      expect(bob.call?.outgoing, isFalse);
      await until(
        () => alice.call?.phase == CallPhase.ringing,
        'Alice hears it ring',
      );

      await bob.acceptCall();
      await until(
        () =>
            alice.call?.phase == CallPhase.active &&
            bob.call?.phase == CallPhase.active,
        'the two devices connect',
      );

      await Future<void>.delayed(const Duration(seconds: 2));
      await alice.hangUp();
      expect(alice.call?.note, CallEndNote.ended);
      await until(
        () => bob.call == null || bob.call?.phase == CallPhase.ended,
        'Bob’s call ends too',
      );
      await until(() => alice.call == null && bob.call == null, 'both close');

      for (final messenger in [alice, bob]) {
        await until(
          () =>
              messenger.chats.single.last?.call?.outcome == CallOutcome.talked,
          'the chat shows the call',
        );
        final call = await lastCall(messenger, chat);
        expect(call?.durationMs, greaterThan(1500));
      }
      expect(bob.chats.single.unread, 0, reason: 'a call picked up is no news');

      // A call turned down.
      await alice.startCall(chat);
      await until(
        () => bob.call?.phase == CallPhase.incoming,
        'it rings again',
      );
      await bob.declineCall();
      await until(
        () => alice.call?.note == CallEndNote.declined,
        'Alice hears that Bob declined',
      );
      await until(() => alice.call == null, 'the screen closes');
      await until(
        () => alice.chats.single.last?.call?.outcome == CallOutcome.declined,
        'the chat says so',
      );
    },
    skip: invites.isEmpty ? 'needs a service and two invites' : null,
    timeout: const Timeout(Duration(minutes: 3)),
  );
}
