import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/app.dart';
import 'package:zelamar/src/calls/call_state.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';

import 'support/fake_messenger.dart';

/// A design review tool, not a check: renders every screen with real fonts
/// into PNG files for people to look at.
///
///     ZELAMAR_TOUR=build/tour flutter test test/tour_test.dart
///
/// Skipped unless ZELAMAR_TOUR names a directory.
final String? _out = Platform.environment['ZELAMAR_TOUR'];
final GlobalKey _shot = GlobalKey();

Future<void> _loadFonts() async {
  Future<void> load(String family, List<String> paths) async {
    final loader = FontLoader(family);
    for (final path in paths) {
      loader.addFont(File(path).readAsBytes().then(ByteData.sublistView));
    }
    await loader.load();
  }

  await load('CormorantGaramond', [
    'assets/fonts/CormorantGaramond-Variable.ttf',
  ]);
  await load('Geist', ['assets/fonts/Geist-Variable.ttf']);
  await load('GeistMono', ['assets/fonts/GeistMono-Variable.ttf']);
  // flutter_tester lives in <flutter>/bin/cache/artifacts/engine/<platform>.
  final cache = File(Platform.resolvedExecutable).parent.parent.parent;
  final material = '${cache.path}/material_fonts';
  await load('MaterialIcons', ['$material/materialicons-regular.otf']);
  await load('Roboto', [
    '$material/roboto-regular.ttf',
    '$material/roboto-medium.ttf',
    '$material/roboto-bold.ttf',
  ]);
}

Future<FakeMessenger> _pump(
  WidgetTester tester,
  FakeMessenger messenger, {
  Size size = const Size(412, 892),
  Brightness brightness = Brightness.light,
}) async {
  tester.view.physicalSize = size * 2;
  tester.view.devicePixelRatio = 2;
  tester.platformDispatcher.platformBrightnessTestValue = brightness;
  addTearDown(tester.view.reset);
  addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
  await tester.pumpWidget(
    RepaintBoundary(
      key: _shot,
      child: ZelamarApp(messenger: messenger),
    ),
  );
  await tester.pumpAndSettle();
  return messenger;
}

Future<void> _capture(WidgetTester tester, String name) async {
  await tester.pump(const Duration(milliseconds: 400));
  final boundary =
      _shot.currentContext!.findRenderObject()! as RenderRepaintBoundary;
  await tester.runAsync(() async {
    final image = await boundary.toImage(pixelRatio: 1.5);
    final png = await image.toByteData(format: ui.ImageByteFormat.png);
    final file = File('$_out/$name.png')..createSync(recursive: true);
    await file.writeAsBytes(png!.buffer.asUint8List());
  });
}

Future<void> _tap(WidgetTester tester, Finder finder) async {
  await tester.tap(finder.first);
  await tester.pumpAndSettle();
}

int _ago(Duration duration) =>
    DateTime.now().subtract(duration).millisecondsSinceEpoch;

/// A believable account: a few people, a group, a request, files.
FakeMessenger _lived({bool recoveryKey = true}) {
  final messenger = FakeMessenger(
    phaseValue: MessengerPhase.ready,
    usernameValue: 'ada',
  )..recoveryKeyValue = recoveryKey ? 'abcde-fghij-klmno-pqrst-uvwxy' : null;
  final photo = FileView(
    name: 'Sea.jpg',
    mime: 'image/jpeg',
    size: BigInt.from(820 * 1024),
    width: 1600,
    height: 1200,
  );
  final voice = FileView(
    name: 'Voice message.m4a',
    mime: 'audio/mp4',
    size: BigInt.from(48 * 1024),
    durationMs: 14000,
  );
  final plan = FileView(
    name: 'Trip plan.pdf',
    mime: 'application/pdf',
    size: BigInt.from(2400 * 1024),
  );
  messenger
    ..addChat(
      'c-bob',
      '@bob',
      unread: 2,
      messages: [
        view(
          9,
          'See you at 7 then?',
          sentAtMs: _ago(const Duration(minutes: 2)),
        ),
        view(8, '', file: voice, sentAtMs: _ago(const Duration(minutes: 3))),
        view(
          7,
          'Perfect, booked it',
          outgoing: true,
          status: MessageStatus.read,
          sentAtMs: _ago(const Duration(minutes: 20)),
          reactions: const [Reaction(emoji: '👍', count: 1, mine: false)],
        ),
        view(
          6,
          'The place by the water has a table',
          sentAtMs: _ago(const Duration(minutes: 25)),
          replyTo: const Quote(
            id: 'm5',
            text: 'Where should we go on Friday?',
            outgoing: true,
          ),
        ),
        view(
          5,
          'Where should we go on Friday?',
          outgoing: true,
          status: MessageStatus.read,
          sentAtMs: _ago(const Duration(hours: 26)),
        ),
        view(4, '', file: plan, sentAtMs: _ago(const Duration(hours: 27))),
      ],
    )
    ..addChat(
      'c-pier',
      'Pier crew',
      group: true,
      messages: [
        view(
          4,
          'Bringing the boat at 9',
          author: '@carol',
          sentAtMs: _ago(const Duration(hours: 1)),
        ),
        view(
          3,
          '',
          file: photo,
          outgoing: true,
          status: MessageStatus.delivered,
          sentAtMs: _ago(const Duration(hours: 2)),
        ),
        view(
          2,
          '',
          author: '@dave',
          event: const ChatEvent(kind: ChatEventKind.left),
          sentAtMs: _ago(const Duration(hours: 3)),
        ),
        view(
          1,
          '',
          author: 'You',
          event: const ChatEvent(kind: ChatEventKind.named, name: 'Pier crew'),
          sentAtMs: _ago(const Duration(hours: 4)),
        ),
      ],
    )
    ..addChat(
      'c-eve',
      '@eve',
      messages: [
        view(
          3,
          '',
          outgoing: true,
          call: const CallView(
            video: false,
            outcome: CallOutcome.talked,
            durationMs: 754000,
          ),
          sentAtMs: _ago(const Duration(days: 2, hours: 1)),
        ),
        view(
          2,
          '',
          call: const CallView(video: true, outcome: CallOutcome.missed),
          sentAtMs: _ago(const Duration(days: 2, hours: 3)),
        ),
        view(
          1,
          'Thanks for the recommendation!',
          outgoing: true,
          status: MessageStatus.delivered,
          sentAtMs: _ago(const Duration(days: 3)),
        ),
      ],
    )
    ..addChat(
      'c-stranger',
      '@mallory_x',
      verified: false,
      request: true,
      messages: [
        view(
          1,
          'Hi! We met at the conference',
          author: '@mallory_x',
          sentAtMs: _ago(const Duration(minutes: 40)),
        ),
      ],
    );
  // A 4x3 sand-coloured picture stands in for the photo.
  messenger.fileBytes['m3'] = base64Decode(
    'iVBORw0KGgoAAAANSUhEUgAAAAQAAAADCAIAAAA7ljmRAAAAEUlEQVR4nGM4uWkyHDHg'
    '5AAA44oYqbXBrRcAAAAASUVORK5CYII=',
  );
  messenger.membersOf['c-pier'] = const [
    MemberView(title: '@bob', verified: true),
    MemberView(title: '@carol', verified: true),
    MemberView(title: '@dave', verified: false),
  ];
  return messenger;
}

void main() {
  final skip = _out == null ? 'set ZELAMAR_TOUR to a directory' : null;

  setUpAll(() async {
    if (_out != null) {
      await _loadFonts();
    }
  });

  group('tour', skip: skip, () {
    testWidgets('onboarding', (tester) async {
      await _pump(tester, FakeMessenger());
      await _capture(tester, '01-welcome');
      await _tap(tester, find.byKey(const Key('welcome.create')));
      await _capture(tester, '02-choose-name');
      await tester.pageBack();
      await tester.pumpAndSettle();
      await _tap(tester, find.byKey(const Key('welcome.join')));
      await _capture(tester, '03-link-this-device');
      await _tap(tester, find.byKey(const Key('join.recover')));
      await _capture(tester, '04-recover');
    });

    testWidgets('first chats, before a recovery key', (tester) async {
      await _pump(
        tester,
        FakeMessenger(phaseValue: MessengerPhase.ready, usernameValue: 'ada'),
      );
      await _capture(tester, '05-chats-new-account');
    });

    testWidgets('chats and a conversation', (tester) async {
      final messenger = await _pump(tester, _lived());
      await _capture(tester, '06-chats');
      await _tap(tester, find.text('@bob'));
      await _capture(tester, '07-chat');
      await tester.longPress(find.text('See you at 7 then?'));
      await tester.pumpAndSettle();
      await _capture(tester, '08-message-actions');
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();
      await _tap(tester, find.byKey(const Key('chat.menu')));
      await _capture(tester, '09-chat-menu');
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();
      await tester.pageBack();
      await tester.pumpAndSettle();
      await _tap(tester, find.text('Pier crew'));
      await _capture(tester, '10-group-chat');
      await _tap(tester, find.byKey(const Key('chat.menu')));
      await _tap(tester, find.byKey(const Key('chat.details')));
      await _capture(tester, '11-group-details');
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.pageBack();
      await tester.pumpAndSettle();
      await _tap(tester, find.text('@mallory_x'));
      await _capture(tester, '12-chat-request');
      expect(messenger.chats, isNotEmpty);
    });

    testWidgets('new chat and new group', (tester) async {
      await _pump(tester, _lived());
      await _tap(tester, find.byKey(const Key('chats.new')));
      await _capture(tester, '13-new-chat');
      await _tap(tester, find.byKey(const Key('newChat.mode')));
      await _capture(tester, '14-new-group');
    });

    testWidgets('you, devices, recovery key', (tester) async {
      await _pump(tester, _lived()..startsAtSignInValue = true);
      await _tap(tester, find.byKey(const Key('chats.profile')));
      await _capture(tester, '15-you');
      await _tap(tester, find.byKey(const Key('profile.devices')));
      await _capture(tester, '16-devices');
      await tester.tap(find.byKey(const Key('devices.link')));
      // The link screen waits with a spinner: no settling.
      for (var frame = 0; frame < 10; frame++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      await _capture(tester, '17-link-a-device');
      await tester.pageBack();
      for (var frame = 0; frame < 10; frame++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      await tester.pageBack();
      await tester.pumpAndSettle();
      await _tap(tester, find.byKey(const Key('profile.recovery')));
      await _capture(tester, '18-recovery-key');
    });

    testWidgets('recovery key, first time', (tester) async {
      await _pump(tester, _lived(recoveryKey: false));
      await _tap(tester, find.byKey(const Key('chats.recovery')));
      await _capture(tester, '19-recovery-key-none');
    });

    testWidgets('safety code', (tester) async {
      await _pump(tester, _lived());
      await _tap(tester, find.text('@bob'));
      await _tap(tester, find.byKey(const Key('chat.menu')));
      await _tap(tester, find.byKey(const Key('chat.safety')));
      await _capture(tester, '20-safety-code');
    });

    testWidgets('a removed device', (tester) async {
      await _pump(
        tester,
        FakeMessenger(
          phaseValue: MessengerPhase.unlinked,
          usernameValue: 'ada',
        ),
      );
      await _capture(tester, '21-unlinked');
    });

    testWidgets('offline, dark and narrow', (tester) async {
      await _pump(
        tester,
        _lived()..linkValue = Link.offline,
        brightness: Brightness.dark,
      );
      await _capture(tester, '22-chats-dark-offline');
      await _tap(tester, find.text('@bob'));
      await _capture(tester, '23-chat-dark');
    });

    testWidgets('a small phone', (tester) async {
      await _pump(tester, _lived(), size: const Size(360, 640));
      await _capture(tester, '24-chats-small');
      await _tap(tester, find.text('@bob'));
      await _capture(tester, '25-chat-small');
    });

    testWidgets('calls', (tester) async {
      final messenger = await _pump(tester, _lived());
      await _tap(tester, find.text('@eve'));
      await _capture(tester, '26-call-lines');
      const incoming = CallState(
        chat: 'c-eve',
        peer: '@eve',
        video: false,
        outgoing: false,
        phase: CallPhase.incoming,
        id: 'call',
      );
      messenger.showCall(incoming);
      await tester.pump();
      await _capture(tester, '27-call-incoming');
      messenger.showCall(
        const CallState(
          chat: 'c-eve',
          peer: '@eve',
          video: false,
          outgoing: true,
          phase: CallPhase.ringing,
          id: 'call',
        ),
      );
      await tester.pump();
      await _capture(tester, '28-call-ringing');
      messenger.showCall(
        incoming.copyWith(
          phase: CallPhase.active,
          connectedAt: DateTime.now().subtract(
            const Duration(minutes: 3, seconds: 24),
          ),
          muted: true,
        ),
      );
      await tester.pump();
      await _capture(tester, '29-call-active');
      messenger.showCall(null);
      await tester.pump(const Duration(milliseconds: 400));
    });

    testWidgets('a call at night, on a small phone', (tester) async {
      final messenger = await _pump(
        tester,
        _lived(),
        size: const Size(360, 640),
        brightness: Brightness.dark,
      );
      messenger.showCall(
        const CallState(
          chat: 'c-bob',
          peer: '@bob',
          video: true,
          outgoing: false,
          phase: CallPhase.incoming,
          id: 'call',
        ),
      );
      await tester.pump();
      await _capture(tester, '30-call-dark-small');
      messenger.showCall(null);
      await tester.pump(const Duration(milliseconds: 400));
    });
  });
}
