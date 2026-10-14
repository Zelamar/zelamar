import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/app.dart';
import 'package:zelamar/src/lab/lab_engine.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/words.dart';

import 'support/fake_messenger.dart';

Future<LabEngine> _noLab() =>
    Future.error(UnsupportedError('no lab in these tests'));

Future<FakeMessenger> pumpApp(
  WidgetTester tester, {
  FakeMessenger? messenger,
  Size size = const Size(420, 860),
}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final fake = messenger ?? FakeMessenger();
  await tester.pumpWidget(ZelamarApp(messenger: fake, startLab: _noLab));
  await tester.pumpAndSettle();
  return fake;
}

void main() {
  group('onboarding', () {
    testWidgets('welcome leads to a name, and the name to the chats', (
      tester,
    ) async {
      final messenger = await pumpApp(tester);
      expect(find.text('Private by design.'), findsOneWidget);

      await tester.tap(find.byKey(const Key('welcome.create')));
      await tester.pumpAndSettle();
      expect(find.text('Choose your name'), findsOneWidget);

      final next = find.byKey(const Key('account.continue'));
      expect(tester.widget<FilledButton>(next).onPressed, isNull);

      await tester.enterText(find.byKey(const Key('account.username')), '9ab');
      await tester.pump();
      expect(
        tester.widget<Text>(find.byKey(const Key('account.rule'))).data,
        describeFailure('invalid_username'),
      );

      await tester.enterText(
        find.byKey(const Key('account.username')),
        'Ada_92',
      );
      await tester.enterText(find.byKey(const Key('account.invite')), 'abc');
      await tester.pump();
      await tester.tap(next);
      await tester.pumpAndSettle();

      expect(messenger.username, 'ada_92');
      expect(find.text('Chats'), findsOneWidget);
      expect(find.text('@ada_92'), findsOneWidget);
      expect(find.text('Your conversations live here.'), findsOneWidget);
    });

    testWidgets('a refused name is explained, and the form stays', (
      tester,
    ) async {
      final messenger = await pumpApp(tester)
        ..failNext = const MessengerFailure('username_taken');
      await tester.tap(find.byKey(const Key('welcome.create')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('account.username')), 'ada');
      await tester.enterText(find.byKey(const Key('account.invite')), 'abc');
      await tester.pump();
      await tester.tap(find.byKey(const Key('account.continue')));
      await tester.pumpAndSettle();

      expect(find.text(describeFailure('username_taken')), findsOneWidget);
      expect(find.text('Choose your name'), findsOneWidget);
      expect(messenger.phase, MessengerPhase.needsAccount);
    });

    testWidgets('fits a narrow phone without overflow', (tester) async {
      await pumpApp(tester, size: const Size(320, 568));
      await tester.tap(find.byKey(const Key('welcome.create')));
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
    });
  });

  group('chats', () {
    FakeMessenger ready() =>
        FakeMessenger(phaseValue: MessengerPhase.ready, usernameValue: 'ada');

    testWidgets('list names, previews and unconfirmed names', (tester) async {
      final messenger = ready()
        ..addChat(
          'c1',
          '@bob',
          messages: [view(2, 'See you at eight', outgoing: true)],
        )
        ..addChat(
          'c2',
          '@carol',
          verified: false,
          messages: [view(1, 'Hi Ada')],
        );
      await pumpApp(tester, messenger: messenger);

      expect(find.text('@bob'), findsOneWidget);
      expect(find.text('You: See you at eight'), findsOneWidget);
      expect(find.text('@carol'), findsOneWidget);
      expect(find.text('Hi Ada'), findsOneWidget);
      expect(find.text('unconfirmed'), findsOneWidget);
    });

    testWidgets('a new chat by exact name opens the conversation', (
      tester,
    ) async {
      final messenger = ready();
      await pumpApp(tester, messenger: messenger);
      await tester.tap(find.byKey(const Key('chats.new')));
      await tester.pumpAndSettle();

      await tester.enterText(find.byKey(const Key('newChat.username')), 'ada');
      await tester.pump();
      await tester.tap(find.byKey(const Key('newChat.start')));
      await tester.pump();
      expect(find.byKey(const Key('newChat.error')), findsOneWidget);

      messenger.failNext = const MessengerFailure('unknown_account');
      await tester.enterText(
        find.byKey(const Key('newChat.username')),
        'nobody',
      );
      await tester.pump();
      await tester.tap(find.byKey(const Key('newChat.start')));
      await tester.pumpAndSettle();
      expect(find.text(describeFailure('unknown_account')), findsOneWidget);

      await tester.enterText(find.byKey(const Key('newChat.username')), 'Bob');
      await tester.pump();
      await tester.tap(find.byKey(const Key('newChat.start')));
      await tester.pumpAndSettle();
      expect(
        tester.widget<Text>(find.byKey(const Key('chat.title'))).data,
        '@bob',
      );
    });
  });

  group('a conversation', () {
    Future<FakeMessenger> openChat(WidgetTester tester) async {
      final yesterday = DateTime.now()
          .subtract(const Duration(days: 1))
          .millisecondsSinceEpoch;
      final messenger =
          FakeMessenger(phaseValue: MessengerPhase.ready, usernameValue: 'ada')
            ..addChat(
              'c1',
              '@bob',
              messages: [
                view(3, 'Perfect', outgoing: true, status: MessageStatus.read),
                view(2, 'Dinner at eight?'),
                view(1, 'Hello', sentAtMs: yesterday),
              ],
            );
      await pumpApp(tester, messenger: messenger);
      await tester.tap(find.byKey(const Key('chats.tile.c1')));
      await tester.pumpAndSettle();
      return messenger;
    }

    testWidgets('shows bubbles, days and read receipts', (tester) async {
      await openChat(tester);
      expect(find.text('Dinner at eight?'), findsOneWidget);
      expect(find.text('Perfect'), findsOneWidget);
      expect(find.text('Yesterday'), findsOneWidget);
      expect(find.text('Today'), findsOneWidget);
      expect(find.byKey(const Key('chat.status.read')), findsOneWidget);
    });

    testWidgets('sends with the button and with Enter', (tester) async {
      final messenger = await openChat(tester);
      final input = find.byKey(const Key('chat.input'));

      await tester.enterText(input, '  On my way  ');
      await tester.pump();
      await tester.tap(find.byKey(const Key('chat.send')));
      await tester.pumpAndSettle();
      expect(messenger.sent, ['c1: On my way']);
      expect(find.text('On my way'), findsOneWidget);
      expect(find.byKey(const Key('chat.status.queued')), findsOneWidget);

      await tester.enterText(input, 'See you');
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(messenger.sent.last, 'c1: See you');
      expect(
        tester.widget<TextField>(input).controller?.text,
        isEmpty,
        reason: 'the field is cleared after sending',
      );
    });
  });
}
