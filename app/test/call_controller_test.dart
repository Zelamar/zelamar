import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/calls/call_controller.dart';
import 'package:zelamar/src/calls/call_state.dart';
import 'package:zelamar/src/calls/media_link.dart';
import 'package:zelamar/src/rust/api/session.dart';

/// A call id: 16 bytes of [n], base64url.
String id(int n) => base64Url.encode(List.filled(16, n)).replaceAll('=', '');

class FakeLink implements MediaLink {
  final stateEvents = StreamController<MediaState>.broadcast();
  final log = <String>[];
  bool closed = false;

  @override
  Stream<MediaState> get states => stateEvents.stream;

  @override
  Future<String> offer({required bool video}) async {
    log.add('offer video=$video');
    return 'offer-sdp';
  }

  @override
  Future<String> answer(String offer, {required bool video}) async {
    log.add('answer $offer');
    return 'answer-sdp';
  }

  @override
  Future<void> accept(String answer) async => log.add('accept $answer');

  @override
  Future<void> setMuted({required bool muted}) async => log.add('muted=$muted');

  @override
  Future<void> setSpeaker({required bool on}) async => log.add('speaker=$on');

  @override
  Future<void> close() async {
    closed = true;
    await stateEvents.close();
  }
}

class FakeSignaling implements CallSignaling {
  final sent = <String>[];
  String nextId = id(9);
  List<IceServerView>? serversAsked;

  @override
  Future<String> offer(
    String chat, {
    required bool video,
    required String sdp,
  }) async {
    sent.add('offer $chat $sdp');
    return nextId;
  }

  @override
  Future<void> answer(String chat, String call, String sdp) async =>
      sent.add('answer $call $sdp');

  @override
  Future<void> end(
    String chat,
    String call,
    CallEnding ending, {
    int? durationMs,
  }) async => sent.add('end $call ${ending.name} $durationMs');

  @override
  Future<List<IceServerView>> servers() async => const [
    IceServerView(urls: ['stun:stun.example:3478']),
  ];
}

class CountingRinger implements Ringer {
  final sounds = <String>[];

  @override
  void ring() => sounds.add('ring');

  @override
  void ringBack() => sounds.add('ring-back');

  @override
  void stop() => sounds.add('stop');
}

class Rig {
  new() {
    controller = CallController(
      signaling: signaling,
      links: (servers) {
        serversGiven = servers;
        final link = FakeLink();
        links.add(link);
        return link;
      },
      ringer: ringer,
      clock: () => now,
    )..onRing = rang.add;
  }

  final signaling = FakeSignaling();
  final ringer = CountingRinger();
  final links = <FakeLink>[];
  final rang = <CallState>[];
  List<IceServerView>? serversGiven;
  DateTime now = DateTime.utc(2026, 10, 27, 12);
  late final CallController controller;

  CallState? get state => controller.state;
  FakeLink get link => links.last;

  CallSignal signal(
    CallSignalKind kind,
    String call, {
    bool own = false,
    bool ring = true,
    String? sdp,
    CallEnding? ending,
  }) => CallSignal(
    kind: kind,
    chat: 'chat',
    call: call,
    own: own,
    peer: '@bob',
    video: false,
    sdp: sdp ?? (kind == CallSignalKind.offer ? 'their-offer' : null),
    ring: ring,
    ending: ending,
  );
}

void main() {
  testWidgets('a call goes out, rings, connects, and ends', (tester) async {
    final rig = Rig();
    unawaited(rig.controller.start('chat', peer: '@bob', video: false));
    expect(rig.state?.phase, CallPhase.calling);
    await tester.pump();
    expect(rig.signaling.sent, ['offer chat offer-sdp']);
    expect(rig.serversGiven?.single.urls, ['stun:stun.example:3478']);
    expect(rig.state?.id, id(9));

    rig.controller.handle(rig.signal(CallSignalKind.ringing, id(9)));
    expect(rig.state?.phase, CallPhase.ringing);
    expect(rig.ringer.sounds.last, 'ring-back');

    rig.controller.handle(
      rig.signal(CallSignalKind.answer, id(9), sdp: 'their-answer'),
    );
    await tester.pump();
    expect(rig.state?.phase, CallPhase.connecting);
    expect(rig.link.log, contains('accept their-answer'));
    expect(rig.ringer.sounds.last, 'stop');

    rig.link.stateEvents.add(MediaState.connected);
    await tester.pump();
    expect(rig.state?.phase, CallPhase.active);

    await rig.controller.setMuted(muted: true);
    expect(rig.state?.muted, isTrue);
    expect(rig.link.log, contains('muted=true'));

    rig.now = rig.now.add(const Duration(minutes: 3, seconds: 24));
    final link = rig.link;
    await rig.controller.hangUp();
    expect(rig.signaling.sent.last, 'end ${id(9)} hangup 204000');
    expect(rig.state?.phase, CallPhase.ended);
    expect(rig.state?.note, CallEndNote.ended);
    expect(link.closed, isTrue);

    await tester.pump(const Duration(seconds: 2));
    expect(rig.state, isNull, reason: 'the ended screen goes away');
  });

  testWidgets('a call nobody picks up gives up after 45 seconds', (
    tester,
  ) async {
    final rig = Rig();
    unawaited(rig.controller.start('chat', peer: '@bob', video: true));
    await tester.pump();
    expect(rig.link.log, ['offer video=true']);
    await tester.pump(const Duration(seconds: 44));
    expect(rig.state?.phase, CallPhase.calling);
    await tester.pump(const Duration(seconds: 1));
    expect(rig.signaling.sent.last, 'end ${id(9)} missed null');
    expect(rig.state?.note, CallEndNote.noAnswer);
    await tester.pump(const Duration(seconds: 2));
  });

  testWidgets('an incoming call rings, is picked up, and ends from afar', (
    tester,
  ) async {
    final rig = Rig();
    rig.controller.handle(rig.signal(CallSignalKind.offer, id(1)));
    expect(rig.state?.phase, CallPhase.incoming);
    expect(rig.state?.outgoing, isFalse);
    expect(rig.rang.single.peer, '@bob');
    expect(rig.ringer.sounds, contains('ring'));

    unawaited(rig.controller.accept());
    expect(rig.state?.phase, CallPhase.connecting);
    await tester.pump();
    expect(rig.link.log, ['answer their-offer']);
    expect(rig.signaling.sent, ['answer ${id(1)} answer-sdp']);

    rig.link.stateEvents.add(MediaState.connected);
    await tester.pump();
    expect(rig.state?.phase, CallPhase.active);

    rig.controller.handle(
      rig.signal(CallSignalKind.end, id(1), ending: CallEnding.hangup),
    );
    expect(rig.state?.note, CallEndNote.ended);
    expect(rig.signaling.sent, hasLength(1), reason: 'nothing more to say');
    await tester.pump(const Duration(seconds: 2));
  });

  testWidgets('only a fresh call from someone else rings', (tester) async {
    final rig = Rig();
    rig.controller
      ..handle(rig.signal(CallSignalKind.offer, id(1), ring: false))
      ..handle(rig.signal(CallSignalKind.offer, id(2), own: true));
    expect(rig.state, isNull);
    expect(rig.ringer.sounds, isEmpty);
  });

  testWidgets('a ringing phone stops when another device picks up', (
    tester,
  ) async {
    final rig = Rig();
    rig.controller
      ..handle(rig.signal(CallSignalKind.offer, id(1)))
      ..handle(rig.signal(CallSignalKind.answer, id(1), own: true, sdp: 'x'));
    expect(rig.state, isNull);
    expect(rig.ringer.sounds.last, 'stop');
    expect(rig.signaling.sent, isEmpty);

    // Declined on another device, too.
    rig.controller
      ..handle(rig.signal(CallSignalKind.offer, id(2)))
      ..handle(
        rig.signal(
          CallSignalKind.end,
          id(2),
          own: true,
          ending: CallEnding.declined,
        ),
      );
    expect(rig.state, isNull);
  });

  testWidgets('declining tells the caller', (tester) async {
    final rig = Rig();
    rig.controller.handle(rig.signal(CallSignalKind.offer, id(1)));
    await rig.controller.decline();
    expect(rig.signaling.sent, ['end ${id(1)} declined null']);
    expect(rig.state, isNull);
  });

  testWidgets('a ringing phone stops by itself after a minute', (tester) async {
    final rig = Rig();
    rig.controller.handle(rig.signal(CallSignalKind.offer, id(1)));
    await tester.pump(const Duration(seconds: 60));
    expect(rig.state, isNull);
    expect(rig.signaling.sent, isEmpty, reason: 'the caller says it');
  });

  testWidgets('the caller hears declined, no answer and busy', (tester) async {
    for (final (ending, note) in [
      (CallEnding.declined, CallEndNote.declined),
      (CallEnding.missed, CallEndNote.noAnswer),
      (CallEnding.busy, CallEndNote.busy),
    ]) {
      final rig = Rig();
      unawaited(rig.controller.start('chat', peer: '@bob', video: false));
      await tester.pump();
      rig.controller.handle(
        rig.signal(CallSignalKind.end, id(9), ending: ending),
      );
      expect(rig.state?.note, note);
      expect(rig.signaling.sent, hasLength(1));
      await tester.pump(const Duration(seconds: 2));
    }
  });

  testWidgets('calling each other at once: the smaller id wins', (
    tester,
  ) async {
    // Ours is bigger: we withdraw ours and pick theirs up.
    final rig = Rig();
    unawaited(rig.controller.start('chat', peer: '@bob', video: false));
    await tester.pump();
    rig.controller.handle(rig.signal(CallSignalKind.offer, id(1)));
    await tester.pump();
    expect(rig.signaling.sent, [
      'offer chat offer-sdp',
      'end ${id(9)} hangup null',
      'answer ${id(1)} answer-sdp',
    ]);
    expect(rig.state?.id, id(1));
    expect(rig.state?.phase, CallPhase.connecting);
    expect(rig.links.first.closed, isTrue);

    // Ours is smaller: theirs is ignored, they pick ours up.
    final other = Rig()..signaling.nextId = id(1);
    unawaited(other.controller.start('chat', peer: '@bob', video: false));
    await tester.pump();
    other.controller.handle(other.signal(CallSignalKind.offer, id(9)));
    await tester.pump();
    expect(other.signaling.sent, ['offer chat offer-sdp']);
    expect(other.state?.id, id(1));
    expect(other.state?.phase, CallPhase.calling);
    await tester.pump(const Duration(seconds: 45));
    await tester.pump(const Duration(seconds: 2));
  });

  testWidgets('another call while talking does not ring', (tester) async {
    final rig = Rig();
    rig.controller.handle(rig.signal(CallSignalKind.offer, id(1)));
    unawaited(rig.controller.accept());
    await tester.pump();
    rig.controller.handle(
      CallSignal(
        kind: CallSignalKind.offer,
        chat: 'another chat',
        call: id(2),
        own: false,
        peer: '@carol',
        video: false,
        sdp: 'offer',
        ring: true,
      ),
    );
    expect(rig.state?.id, id(1));
    expect(rig.rang, hasLength(1));
    await rig.controller.hangUp();
    await tester.pump(const Duration(seconds: 2));
  });

  testWidgets('a lost connection ends the call after 15 seconds', (
    tester,
  ) async {
    final rig = Rig();
    rig.controller.handle(rig.signal(CallSignalKind.offer, id(1)));
    unawaited(rig.controller.accept());
    await tester.pump();
    rig.link.stateEvents.add(MediaState.connected);
    await tester.pump();
    rig.now = rig.now.add(const Duration(seconds: 30));

    // A short gap heals.
    rig.link.stateEvents.add(MediaState.interrupted);
    await tester.pump(const Duration(seconds: 10));
    rig.link.stateEvents.add(MediaState.connected);
    await tester.pump(const Duration(seconds: 10));
    expect(rig.state?.phase, CallPhase.active);

    rig.link.stateEvents.add(MediaState.interrupted);
    await tester.pump(const Duration(seconds: 15));
    expect(rig.state?.note, CallEndNote.failed);
    expect(rig.signaling.sent.last, 'end ${id(1)} failed 30000');
    await tester.pump(const Duration(seconds: 2));
  });

  testWidgets('a connection that never comes up fails after 30 seconds', (
    tester,
  ) async {
    final rig = Rig();
    unawaited(rig.controller.start('chat', peer: '@bob', video: false));
    await tester.pump();
    rig.controller.handle(
      rig.signal(CallSignalKind.answer, id(9), sdp: 'answer'),
    );
    await tester.pump(const Duration(seconds: 30));
    expect(rig.state?.note, CallEndNote.failed);
    expect(rig.signaling.sent.last, 'end ${id(9)} failed null');
    await tester.pump(const Duration(seconds: 2));
  });

  testWidgets('a device that loses the race to pick up leaves quietly', (
    tester,
  ) async {
    final rig = Rig();
    rig.controller.handle(rig.signal(CallSignalKind.offer, id(1)));
    unawaited(rig.controller.accept());
    await tester.pump();
    rig.controller.handle(
      rig.signal(CallSignalKind.answer, id(1), own: true, sdp: 'other'),
    );
    await tester.pump(const Duration(seconds: 30));
    expect(rig.state, isNull);
    expect(rig.signaling.sent, ['answer ${id(1)} answer-sdp']);
  });

  testWidgets('hanging up while the call is on its way withdraws it', (
    tester,
  ) async {
    final rig = Rig();
    unawaited(rig.controller.start('chat', peer: '@bob', video: false));
    await rig.controller.hangUp();
    expect(rig.state, isNull);
    await tester.pump();
    expect(rig.signaling.sent, isEmpty, reason: 'nothing was sent yet');
  });
}
