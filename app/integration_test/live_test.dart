import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/live_messenger.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/notifier.dart';
import 'package:zelamar/src/messenger/outgoing_file.dart';
import 'package:zelamar/src/messenger/transport.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/rust/frb_generated.dart';

/// The whole stack, for real: app instances, each with its own Rust core,
/// encrypted store and secure key, meet through a running delivery service
/// over HTTP and WebSocket; one of them links a second device, and another
/// recovers a lost account with its recovery key.
///
/// Needs a service and four fresh invite codes:
///
///     cd server && npx wrangler dev --port 8787
///     flutter test integration_test/live_test.dart -d windows \
///       --dart-define=ZELAMAR_SERVICE=http://127.0.0.1:8787 \
///       --dart-define=ZELAMAR_TEST_INVITES=<code>,<code>,<code>,<code>
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
  final codes = invites.split(',');
  final run = Random().nextInt(1 << 30).toRadixString(36);

  /// Messengers a test already took down, as a lost device.
  final lost = <LiveMessenger>{};

  /// A fresh app instance with its own store, key and connection.
  Future<LiveMessenger> person(
    String name, {
    Duration memberHealing = const Duration(minutes: 5),
  }) async {
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
      linkPoll: const Duration(milliseconds: 300),
      memberHealing: memberHealing,
    );
    addTearDown(() {
      if (!lost.contains(messenger)) {
        messenger.dispose();
      }
    });
    await messenger.start();
    expect(messenger.phase, MessengerPhase.needsAccount);
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

  test(
    'two people meet by username, talk, and one adds a second device',
    () async {
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

      // Alice links her phone with the code her first device shows.
      final phone = await person('phone');
      expect(alice.canLinkDevices, isTrue);
      final code = await alice.offerLink();
      final asked = alice.nextApplicant();
      String? shown;
      final joined = phone.joinAccount(
        code: code,
        onMatch: (match) => shown = match,
      );
      final applicant = await asked;
      expect(applicant, isNotNull);
      expect(applicant!.matchCode, shown);
      await alice.approveLink();
      await joined;
      expect(phone.phase, MessengerPhase.ready);
      expect(phone.username, aliceName);
      expect(phone.fingerprint, alice.fingerprint);
      expect(phone.canLinkDevices, isFalse);

      // Her first device brings the phone into the chat with Bob.
      await until(
        () => phone.chats.any((c) => c.id == chat && !c.request),
        'the phone joins the chat with Bob',
      );
      await bob.sendText(chat, 'Two devices now?');
      await until(
        () => phone.chats.single.last?.text == 'Two devices now?',
        'the phone receives Bob’s message',
      );
      await phone.sendText(chat, 'Yes, this is my phone.');
      await until(
        () => bob.chats.single.last?.text == 'Yes, this is my phone.',
        'Bob receives the phone’s message',
      );
      expect(bob.chats.single.title, '@$aliceName');
      await until(
        () => alice.chats.single.last?.text == 'Yes, this is my phone.',
        'Alice’s first device sees what the phone wrote',
      );
      expect((await alice.messages(chat)).first.outgoing, isTrue);

      // The phone is lost: Alice removes it, and it hears nothing more.
      final devices = await alice.devices();
      expect(devices.length, 2);
      expect(devices.first.thisDevice, isTrue);
      await alice.removeDevice(devices.last.id);
      await until(
        () => phone.phase == MessengerPhase.unlinked,
        'the phone learns it was removed',
      );
      expect(await alice.devices(), hasLength(1));
      await bob.sendText(chat, 'Only your computer now.');
      await until(
        () => alice.chats.single.last?.text == 'Only your computer now.',
        'Alice still reads Bob',
      );
      expect(phone.chats.single.last?.text, 'Yes, this is my phone.');
      await phone.eraseThisDevice();
      expect(phone.phase, MessengerPhase.needsAccount);

      // A named group: Bob joins, writes, then leaves.
      final crew = await alice.createGroup('QA crew', [bobName]);
      ChatPreview crewAt(LiveMessenger who) =>
          who.chats.firstWhere((c) => c.id == crew);
      await until(
        () => bob.chats.any((c) => c.id == crew && c.title == 'QA crew'),
        'Bob sees the named group',
      );
      expect(crewAt(bob).request, isTrue);
      await bob.acceptChat(crew);
      await bob.sendText(crew, 'In the crew');
      await until(
        () => crewAt(alice).last?.text == 'In the crew',
        'Alice reads Bob in the group',
      );
      await bob.leaveChat(crew);
      expect(bob.chats.any((c) => c.id == crew), isFalse);
      await until(
        () => crewAt(alice).last?.event?.kind == ChatEventKind.left,
        'Alice sees Bob leave',
      );
      // A file travels sealed and opens on the other side.
      final pdf = Uint8List.fromList(
        List.generate(1500 * 1024, (i) => i % 251),
      );
      await alice.sendFile(
        chat,
        OutgoingFile(name: 'Plan.pdf', bytes: pdf, mime: 'application/pdf'),
      );
      await until(
        () => bob.chats.single.last?.file?.name == 'Plan.pdf',
        'Bob receives the file',
      );
      final received = bob.chats.single.last!;
      expect(await bob.openFile(chat, received.id), pdf);

      final deadline = DateTime.now().add(const Duration(seconds: 30));
      while ((await alice.members(crew)).isNotEmpty) {
        if (DateTime.now().isAfter(deadline)) {
          fail('Timed out waiting until Bob’s devices are out of the group');
        }
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
    },
    skip: invites.isEmpty ? 'needs a running service and invites' : false,
    timeout: const Timeout(Duration(minutes: 3)),
  );

  test(
    'a lost account comes back with its recovery key, and its chat with it',
    () async {
      final alice = await person('ann');
      // Bob looks at his contacts' devices often, as an app left running
      // in the tray does every five minutes.
      final bob = await person(
        'ben',
        memberHealing: const Duration(seconds: 6),
      );
      final aliceName = 'qa_${run}_c';
      final bobName = 'qa_${run}_d';
      await alice.createAccount(username: aliceName, invite: codes[2]);
      await bob.createAccount(username: bobName, invite: codes[3]);
      await until(
        () => alice.link == Link.online && bob.link == Link.online,
        'both are online',
      );
      final chat = await alice.startChat(bobName);
      await alice.sendText(chat, 'Hi Ben');
      await until(
        () => bob.chats.any((c) => c.last?.text == 'Hi Ben'),
        'Ben receives the message',
      );
      await bob.sendText(chat, 'Hi Ann');
      await until(
        () => alice.chats.single.last?.text == 'Hi Ann',
        'Ann receives the reply',
      );

      // Ann writes down a recovery key; her account is sealed with it.
      expect(alice.recoveryKey, isNull);
      final key = await alice.newRecoveryKey();
      expect(alice.recoveryKey, key);
      expect(alice.recoveryKeySaved, isTrue);
      final fingerprint = alice.fingerprint;
      final lostDevice = (await alice.devices()).single.id;

      // Her only device is gone.
      lost.add(alice);
      alice.dispose();

      final restored = await person('ann-again');
      await expectLater(
        restored.recoverAccount(username: bobName, key: key),
        throwsA(
          isA<MessengerFailure>().having(
            (f) => f.code,
            'code',
            'wrong_recovery_key',
          ),
        ),
      );
      await restored.recoverAccount(
        username: aliceName.toUpperCase(),
        key: key.toUpperCase().replaceAll('-', ' '),
      );
      expect(restored.phase, MessengerPhase.ready);
      expect(restored.username, aliceName);
      expect(restored.fingerprint, fingerprint);
      expect(restored.canLinkDevices, isTrue);
      expect(restored.recoveryKey, key);

      // She removes the lost device; Ben's app brings her back into the
      // chat, and they talk again.
      final devices = await restored.devices();
      expect(devices.map((d) => d.id), contains(lostDevice));
      await restored.removeDevice(lostDevice);
      await until(
        () => restored.chats.any((c) => c.id == chat && !c.request),
        'Ben brings the new device into the chat',
        within: const Duration(seconds: 60),
      );
      expect(restored.chats.single.title, '@$bobName');
      await bob.sendText(chat, 'Welcome back, Ann');
      await until(
        () => restored.chats.single.last?.text == 'Welcome back, Ann',
        'the new device reads Ben',
      );
      await restored.sendText(chat, 'Thanks, Ben');
      await until(
        () => bob.chats.single.last?.text == 'Thanks, Ben',
        'Ben reads the new device',
      );
      expect(await restored.devices(), hasLength(1));
    },
    skip: codes.length < 4 ? 'needs a running service and four invites' : false,
    timeout: const Timeout(Duration(minutes: 3)),
  );
}
