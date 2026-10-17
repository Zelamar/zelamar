import 'package:flutter/gestures.dart';
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

/// Lets screens with a spinning indicator move on: [WidgetTester.pumpAndSettle]
/// would wait for the spinner forever.
Future<void> pumpAWhile(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 600));
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

    testWidgets('a new device joins with the code its other device shows', (
      tester,
    ) async {
      final messenger = await pumpApp(tester);
      await tester.tap(find.byKey(const Key('welcome.join')));
      await tester.pumpAndSettle();
      expect(find.text('Link this device'), findsOneWidget);

      final next = find.byKey(const Key('join.continue'));
      await tester.enterText(find.byKey(const Key('join.code')), 'abcde fghij');
      await tester.pump();
      expect(tester.widget<FilledButton>(next).onPressed, isNull);

      await tester.enterText(
        find.byKey(const Key('join.code')),
        'ABCDE-fghij klmno-pqrst',
      );
      await tester.pump();
      await tester.tap(next);
      await pumpAWhile(tester);
      expect(messenger.joinedWith, 'ABCDE-fghij klmno-pqrst');
      expect(find.byKey(const Key('join.match')), findsOneWidget);
      expect(find.text('482 913'), findsOneWidget);

      messenger.joinApproved.complete();
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('app.chats')), findsOneWidget);
    });

    testWidgets('a used code is explained, and cancelling goes back', (
      tester,
    ) async {
      final messenger = await pumpApp(tester);
      await tester.tap(find.byKey(const Key('welcome.join')));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('join.code')),
        'abcdefghijklmnopqrst',
      );
      await tester.pump();

      messenger.failNext = const MessengerFailure('link_taken');
      await tester.tap(find.byKey(const Key('join.continue')));
      await tester.pumpAndSettle();
      expect(
        tester.widget<Text>(find.byKey(const Key('join.error'))).data,
        describeFailure('link_taken'),
      );

      await tester.tap(find.byKey(const Key('join.continue')));
      await pumpAWhile(tester);
      await tester.tap(find.byKey(const Key('join.cancel')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('join.form')), findsOneWidget);
      expect(find.byKey(const Key('join.error')), findsNothing);
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
          unread: 3,
        );
      await pumpApp(tester, messenger: messenger);

      expect(find.text('@bob'), findsOneWidget);
      expect(find.text('You: See you at eight'), findsOneWidget);
      expect(find.text('@carol'), findsOneWidget);
      expect(find.text('Hi Ada'), findsOneWidget);
      expect(find.text('unconfirmed'), findsOneWidget);
      expect(find.byKey(const Key('chats.unread')), findsOneWidget);
      expect(find.text('3'), findsOneWidget);

      // Opening the chat reads it; the badge is gone on the way back.
      await tester.tap(find.byKey(const Key('chats.tile.c2')));
      await tester.pumpAndSettle();
      expect(messenger.read, contains('c2'));
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('chats.unread')), findsNothing);
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

  group('groups', () {
    testWidgets('a group is started with a name and people', (tester) async {
      final messenger = await pumpApp(
        tester,
        messenger: FakeMessenger(
          phaseValue: MessengerPhase.ready,
          usernameValue: 'ada',
        ),
      );
      await tester.tap(find.byKey(const Key('chats.new')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('newChat.mode')));
      await tester.pumpAndSettle();
      expect(find.text('New group'), findsOneWidget);
      final create = find.byKey(const Key('newGroup.create'));
      expect(tester.widget<FilledButton>(create).onPressed, isNull);

      await tester.enterText(
        find.byKey(const Key('newGroup.name')),
        'Pier crew',
      );
      for (final name in ['bob', 'Carol', 'bob']) {
        await tester.enterText(find.byKey(const Key('newChat.username')), name);
        await tester.pump();
        await tester.tap(find.byKey(const Key('newGroup.add')));
        await tester.pump();
      }
      expect(find.text('@bob'), findsOneWidget);
      expect(find.text('@carol'), findsOneWidget);
      await tester.enterText(find.byKey(const Key('newChat.username')), 'ada');
      await tester.pump();
      await tester.tap(find.byKey(const Key('newGroup.add')));
      await tester.pump();
      expect(find.byKey(const Key('newChat.error')), findsOneWidget);

      await tester.tap(create);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('chat.title')), findsOneWidget);
      expect(find.text('Pier crew'), findsWidgets);
      expect((await messenger.members('group-0')).map((m) => m.title), [
        '@bob',
        '@carol',
      ]);
    });

    testWidgets('a group shows who wrote what, and what happened', (
      tester,
    ) async {
      final messenger =
          FakeMessenger(phaseValue: MessengerPhase.ready, usernameValue: 'ada')
            ..addChat(
              'g1',
              'Pier crew',
              group: true,
              messages: [
                view(4, 'See you', author: '@carol'),
                view(3, 'Seven?'),
                view(2, 'Hi all'),
                view(
                  1,
                  '',
                  event: const ChatEvent(
                    kind: ChatEventKind.named,
                    name: 'Pier crew',
                  ),
                ),
              ],
            );
      await pumpApp(tester, messenger: messenger);
      await tester.tap(find.byKey(const Key('chats.tile.g1')));
      await tester.pumpAndSettle();
      expect(find.text('@bob named the group “Pier crew”'), findsOneWidget);
      // One name per run of messages from the same person (newest first).
      expect(
        tester
            .widgetList<Text>(find.byKey(const Key('chat.author')))
            .map((t) => t.data),
        ['@carol', '@bob'],
      );
      await tester.tap(find.byKey(const Key('chat.menu')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('chat.safety')), findsNothing);
      expect(find.byKey(const Key('chat.details')), findsOneWidget);
    });

    testWidgets('a group is renamed, grows, and can be left', (tester) async {
      final messenger = FakeMessenger(
        phaseValue: MessengerPhase.ready,
        usernameValue: 'ada',
      )..addChat('g1', 'Pier crew', group: true);
      messenger.membersOf['g1'] = [
        const MemberView(title: '@bob', verified: true),
      ];
      await pumpApp(tester, messenger: messenger);
      await tester.tap(find.byKey(const Key('chats.tile.g1')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('chat.menu')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('chat.details')));
      await tester.pumpAndSettle();
      expect(find.text('PEOPLE · 2'), findsOneWidget);

      await tester.tap(find.byKey(const Key('group.rename')));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('group.field')),
        'Pier crew 🌊',
      );
      await tester.tap(find.byKey(const Key('group.field.done')));
      await tester.pumpAndSettle();
      expect(
        tester.widget<Text>(find.byKey(const Key('group.name'))).data,
        'Pier crew 🌊',
      );

      await tester.tap(find.byKey(const Key('group.add')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('group.field')), 'Dave');
      await tester.tap(find.byKey(const Key('group.field.done')));
      await tester.pumpAndSettle();
      expect(find.text('@dave'), findsOneWidget);
      expect(find.text('PEOPLE · 3'), findsOneWidget);

      await tester.tap(find.byKey(const Key('group.leave')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('group.leave.confirm')));
      await tester.pumpAndSettle();
      expect(messenger.left, ['g1']);
      expect(find.byKey(const Key('chats.tile.g1')), findsNothing);
    });
  });

  group('files', () {
    testWidgets('photos and files show in bubbles', (tester) async {
      final messenger =
          FakeMessenger(phaseValue: MessengerPhase.ready, usernameValue: 'ada')
            ..addChat(
              'c1',
              '@bob',
              messages: [
                view(
                  2,
                  '',
                  file: FileView(
                    name: 'Plan.pdf',
                    mime: 'application/pdf',
                    size: BigInt.from(2516582),
                  ),
                ),
                view(
                  1,
                  '',
                  file: FileView(
                    name: 'pier.jpg',
                    mime: 'image/jpeg',
                    size: BigInt.from(300000),
                    width: 400,
                    height: 300,
                  ),
                ),
              ],
            );
      await pumpApp(tester, messenger: messenger);
      await tester.tap(find.byKey(const Key('chats.tile.c1')));
      await pumpAWhile(tester);
      expect(find.byKey(const Key('chat.image')), findsOneWidget);
      expect(find.byKey(const Key('chat.file')), findsOneWidget);
      expect(find.text('Plan.pdf'), findsOneWidget);
      expect(find.text('2.4 MB'), findsOneWidget);
      expect(find.byKey(const Key('chat.attach')), findsOneWidget);
    });

    testWidgets('voice messages: a microphone when there is no text', (
      tester,
    ) async {
      final messenger =
          FakeMessenger(phaseValue: MessengerPhase.ready, usernameValue: 'ada')
            ..addChat(
              'c1',
              '@bob',
              messages: [
                view(
                  1,
                  '',
                  file: FileView(
                    name: 'Voice message.m4a',
                    mime: 'audio/mp4',
                    size: BigInt.from(24000),
                    durationMs: 7400,
                  ),
                ),
              ],
            );
      await pumpApp(tester, messenger: messenger);
      await tester.tap(find.byKey(const Key('chats.tile.c1')));
      await pumpAWhile(tester);
      expect(find.byKey(const Key('chat.voice.play')), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('chat.voice.time'))).data,
        '0:07',
      );
      expect(find.byKey(const Key('chat.record')), findsOneWidget);
      expect(find.byKey(const Key('chat.send')), findsNothing);
      await tester.enterText(find.byKey(const Key('chat.input')), 'Hi');
      await tester.pump();
      expect(find.byKey(const Key('chat.record')), findsNothing);
      expect(find.byKey(const Key('chat.send')), findsOneWidget);
    });
  });

  group('chat requests', () {
    Future<FakeMessenger> openRequest(WidgetTester tester) async {
      final messenger =
          FakeMessenger(phaseValue: MessengerPhase.ready, usernameValue: 'ada')
            ..addChat('c1', '@bob', messages: [view(1, 'Lunch?')])
            ..addChat(
              'c2',
              '@mallory',
              messages: [view(2, 'Buy now')],
              unread: 1,
              request: true,
            );
      await pumpApp(tester, messenger: messenger);
      expect(find.byKey(const Key('chats.section.requests')), findsOneWidget);
      expect(find.byKey(const Key('chats.section.chats')), findsOneWidget);
      await tester.tap(find.byKey(const Key('chats.tile.c2')));
      await tester.pumpAndSettle();
      expect(find.text('@mallory wants to chat with you.'), findsOneWidget);
      return messenger;
    }

    testWidgets('declining removes the chat after a confirmation', (
      tester,
    ) async {
      final messenger = await openRequest(tester);
      await tester.tap(find.byKey(const Key('chat.decline')));
      await tester.pumpAndSettle();
      expect(find.textContaining('won’t be told'), findsOneWidget);
      await tester.tap(find.byKey(const Key('chat.delete.confirm')));
      await tester.pumpAndSettle();

      expect(messenger.chats.map((chat) => chat.id), ['c1']);
      expect(find.text('Chats'), findsOneWidget);
      expect(find.byKey(const Key('chats.section.requests')), findsNothing);
    });

    testWidgets('accepting turns the request into a chat', (tester) async {
      await openRequest(tester);
      await tester.tap(find.byKey(const Key('chat.accept')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('chat.request')), findsNothing);
    });
  });

  group('you and your keys', () {
    testWidgets('the profile shows the name and the key fingerprint', (
      tester,
    ) async {
      await pumpApp(
        tester,
        messenger: FakeMessenger(
          phaseValue: MessengerPhase.ready,
          usernameValue: 'ada',
        ),
      );
      await tester.tap(find.byKey(const Key('chats.profile')));
      await tester.pumpAndSettle();
      expect(find.text('@ada'), findsWidgets);
      expect(find.byKey(const Key('profile.fingerprint')), findsOneWidget);
      expect(find.textContaining('Zelamar 0.'), findsOneWidget);
    });

    testWidgets('a new device is linked once the numbers match', (
      tester,
    ) async {
      final messenger = await pumpApp(
        tester,
        messenger: FakeMessenger(
          phaseValue: MessengerPhase.ready,
          usernameValue: 'ada',
        ),
      );
      await tester.tap(find.byKey(const Key('chats.profile')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('profile.devices')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('devices.link')));
      await pumpAWhile(tester);
      expect(find.text('abcde-fghij-klmno-pqrst'), findsOneWidget);
      expect(find.text('Waiting for your new device…'), findsOneWidget);

      messenger.applicantArrives.complete(
        const LinkApplicant(label: 'Android phone', matchCode: '482 913'),
      );
      await tester.pumpAndSettle();
      expect(find.text('Link “Android phone”?'), findsOneWidget);
      expect(find.text('482 913'), findsOneWidget);

      await tester.tap(find.byKey(const Key('link.approve')));
      await tester.pumpAndSettle();
      expect(messenger.linkApproved, isTrue);
      expect(find.byKey(const Key('link.done')), findsOneWidget);
      expect(messenger.linkCancelled, isFalse);
    });

    testWidgets('leaving before approving cancels the code', (tester) async {
      final messenger = await pumpApp(
        tester,
        messenger: FakeMessenger(
          phaseValue: MessengerPhase.ready,
          usernameValue: 'ada',
        ),
      );
      await tester.tap(find.byKey(const Key('chats.profile')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('profile.devices')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('devices.link')));
      await pumpAWhile(tester);
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(messenger.linkCancelled, isTrue);
      expect(messenger.linkApproved, isFalse);
    });

    testWidgets('a lost device is removed after a confirmation', (
      tester,
    ) async {
      final messenger = await pumpApp(
        tester,
        messenger: FakeMessenger(
          phaseValue: MessengerPhase.ready,
          usernameValue: 'ada',
        ),
      );
      await tester.tap(find.byKey(const Key('chats.profile')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('profile.devices')));
      await tester.pumpAndSettle();
      expect(find.text('Windows computer'), findsOneWidget);
      expect(find.text('Android phone'), findsOneWidget);
      expect(find.byKey(const Key('devices.remove.d-me')), findsNothing);

      await tester.tap(find.byKey(const Key('devices.remove.d-phone')));
      await tester.pumpAndSettle();
      expect(find.text('Remove “Android phone”?'), findsOneWidget);
      await tester.tap(find.byKey(const Key('devices.confirm')));
      await tester.pumpAndSettle();
      expect(messenger.removedDevices, ['d-phone']);
      expect(find.text('Android phone'), findsNothing);
    });

    testWidgets('a linked device cannot link others, only unlink itself', (
      tester,
    ) async {
      final messenger = await pumpApp(
        tester,
        messenger: FakeMessenger(
          phaseValue: MessengerPhase.ready,
          usernameValue: 'ada',
        )..canLinkValue = false,
      );
      await tester.tap(find.byKey(const Key('chats.profile')));
      await tester.pumpAndSettle();
      expect(
        tester.widget<Text>(find.byKey(const Key('profile.keyNote'))).data,
        contains('this one is linked'),
      );
      await tester.tap(find.byKey(const Key('profile.devices')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('devices.link')), findsNothing);
      expect(find.byKey(const Key('devices.remove.d-phone')), findsNothing);

      await tester.tap(find.byKey(const Key('devices.unlink')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('devices.confirm')));
      await tester.pumpAndSettle();
      expect(messenger.erased, isTrue);
      expect(find.byKey(const Key('app.welcome')), findsOneWidget);
    });

    testWidgets('a device the account removed can only be erased', (
      tester,
    ) async {
      final messenger = await pumpApp(
        tester,
        messenger: FakeMessenger(
          phaseValue: MessengerPhase.unlinked,
          usernameValue: 'ada',
        ),
      );
      expect(find.text('This device was removed from @ada.'), findsOneWidget);
      await tester.tap(find.byKey(const Key('unlinked.erase')));
      await tester.pumpAndSettle();
      expect(messenger.erased, isTrue);
      expect(find.byKey(const Key('app.welcome')), findsOneWidget);
    });

    testWidgets('a chat shows its 60-digit safety code', (tester) async {
      final messenger = FakeMessenger(
        phaseValue: MessengerPhase.ready,
        usernameValue: 'ada',
      )..addChat('c1', '@bob', messages: [view(1, 'Hi')]);
      await pumpApp(tester, messenger: messenger);
      await tester.tap(find.byKey(const Key('chats.tile.c1')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('chat.menu')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('chat.safety')));
      await tester.pumpAndSettle();
      expect(find.text('Safety code with @bob'), findsOneWidget);
      expect(find.text('12345'), findsOneWidget);
      expect(find.text('12356'), findsOneWidget);
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

    Future<void> actionsOn(WidgetTester tester, String id) async {
      await tester.longPress(find.byKey(Key('chat.message.$id')));
      await tester.pumpAndSettle();
    }

    testWidgets('a reaction is set from the actions and removed by a tap', (
      tester,
    ) async {
      final messenger = await openChat(tester);
      await actionsOn(tester, 'm2');
      expect(find.byKey(const Key('actions.edit')), findsNothing);
      await tester.tap(find.byKey(const Key('actions.react.❤️')));
      await tester.pumpAndSettle();
      expect(messenger.sent.last, 'c1: ❤️ m2');
      expect(find.byKey(const Key('chat.reaction.❤️')), findsOneWidget);

      await tester.tap(find.byKey(const Key('chat.reaction.❤️')));
      await tester.pumpAndSettle();
      expect(messenger.sent.last, 'c1: ∅ m2');
      expect(find.byKey(const Key('chat.reaction.❤️')), findsNothing);
    });

    testWidgets('a reply shows what it answers', (tester) async {
      final messenger = await openChat(tester);
      await actionsOn(tester, 'm2');
      await tester.tap(find.byKey(const Key('actions.reply')));
      await tester.pumpAndSettle();
      expect(find.text('Replying to @bob'), findsOneWidget);

      await tester.enterText(find.byKey(const Key('chat.input')), 'Yes!');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(messenger.sent.last, 'c1: ↩ Yes!');
      expect(find.byKey(const Key('chat.banner')), findsNothing);
      expect(find.byKey(const Key('chat.quote')), findsOneWidget);
      expect(find.text('Dinner at eight?'), findsNWidgets(2));
    });

    testWidgets('own messages can be edited, and Escape cancels', (
      tester,
    ) async {
      final messenger = await openChat(tester);
      // Right click, as on a desktop.
      await tester.tap(
        find.byKey(const Key('chat.message.m3')),
        buttons: kSecondaryMouseButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('actions.edit')));
      await tester.pumpAndSettle();
      final input = find.byKey(const Key('chat.input'));
      expect(tester.widget<TextField>(input).controller?.text, 'Perfect');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('chat.banner')), findsNothing);
      expect(tester.widget<TextField>(input).controller?.text, isEmpty);

      await actionsOn(tester, 'm3');
      await tester.tap(find.byKey(const Key('actions.edit')));
      await tester.pumpAndSettle();
      await tester.enterText(input, 'Perfect, see you');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(messenger.sent.last, 'c1: ✎ Perfect, see you');
      expect(find.text('Perfect, see you'), findsOneWidget);
      expect(find.textContaining('edited'), findsOneWidget);
    });

    testWidgets('deleting for everyone asks first', (tester) async {
      final messenger = await openChat(tester);
      await actionsOn(tester, 'm3');
      await tester.tap(find.byKey(const Key('actions.delete')));
      await tester.pumpAndSettle();
      expect(find.text('Delete for everyone?'), findsOneWidget);
      await tester.tap(find.byKey(const Key('message.delete.confirm')));
      await tester.pumpAndSettle();
      expect(messenger.sent.last, 'c1: ✕ m3');
      expect(find.text('Message deleted'), findsOneWidget);
    });

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
