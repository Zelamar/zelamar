import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/widgets.dart';
import 'package:zelamar/src/calls/call_state.dart';
import 'package:zelamar/src/calls/media_link.dart';
import 'package:zelamar/src/rust/api/session.dart';

/// What a call needs from the messenger: its messages, sent through the
/// chat, and the servers that help it connect.
abstract interface class CallSignaling {
  /// Sends the offer into [chat]; returns the call's id.
  Future<String> offer(String chat, {required bool video, required String sdp});

  /// Picks up [call] in [chat] with this device's [sdp].
  Future<void> answer(String chat, String call, String sdp);

  /// Ends [call] in [chat], or turns it down.
  Future<void> end(
    String chat,
    String call,
    CallEnding ending, {
    int? durationMs,
  });

  /// The servers for one call.
  Future<List<IceServerView>> servers();
}

/// The sounds of a ringing call.
abstract interface class Ringer {
  /// Someone is calling this device.
  void ring();

  /// This device is calling, and the other phone rings.
  void ringBack();

  /// Quiet.
  void stop();
}

/// A ringer that makes no sound, for tests and platforms without one.
class SilentRinger implements Ringer {
  const new();

  @override
  void ring() {}

  @override
  void ringBack() {}

  @override
  void stop() {}
}

/// Used when the service cannot say which servers to use: most calls
/// connect with STUN alone.
const _fallbackServers = [
  IceServerView(urls: ['stun:stun.cloudflare.com:3478']),
];

/// Runs this device's calls, one at a time (ADR-0019, `protocol/calls.md`).
///
/// It turns what the user does (call, pick up, hang up) and what arrives
/// through the chat ([handle]) into a [CallState] the call screen shows,
/// and keeps the deadlines: a caller gives up after [noAnswer], a ringing
/// device stops after [ringFor], a connection gets [connectFor] to come up
/// and [lostFor] to come back.
class CallController extends ChangeNotifier {
  new({
    required this.signaling,
    required this.links,
    this.ringer = const SilentRinger(),
    this.noAnswer = const Duration(seconds: 45),
    this.ringFor = const Duration(seconds: 60),
    this.connectFor = const Duration(seconds: 30),
    this.lostFor = const Duration(seconds: 15),
    this.endedFor = const Duration(seconds: 2),
    DateTime Function()? clock,
  }) : _clock = clock ?? DateTime.now;

  final CallSignaling signaling;
  final MediaLinkFactory links;
  final Ringer ringer;
  final Duration noAnswer;
  final Duration ringFor;
  final Duration connectFor;
  final Duration lostFor;
  final Duration endedFor;
  final DateTime Function() _clock;

  /// Called when a call starts ringing on this device.
  void Function(CallState call)? onRing;

  CallState? _state;

  /// Bumped for every new call, so the work of an older one stops.
  int _generation = 0;
  MediaLink? _link;
  StreamSubscription<MediaState>? _media;

  /// The offer of the incoming call.
  String? _offer;

  /// The deadline of the current phase.
  Timer? _deadline;
  Timer? _lost;

  /// Another device of this account picked up the same call too.
  bool _rival = false;

  /// Stops following the current link's pictures.
  void Function()? _unfollow;

  /// The other person's picture in a video call, once it arrives.
  final remoteVideo = ValueNotifier<Widget?>(null);

  /// This device's camera in a video call, while it is on.
  final localVideo = ValueNotifier<Widget?>(null);

  /// The call ringing or going on, or one that just ended.
  CallState? get state => _state;

  /// Whether a call is ringing or going on.
  bool get busy => _state != null && _state!.phase != CallPhase.ended;

  /// Calls the other person in [chat], who is [peer].
  Future<void> start(
    String chat, {
    required String peer,
    required bool video,
  }) async {
    if (busy) {
      return;
    }
    final generation = _begin(
      CallState(
        chat: chat,
        peer: peer,
        video: video,
        outgoing: true,
        phase: CallPhase.calling,
      ),
    );
    try {
      final link = await _connect(generation);
      if (link == null) {
        return;
      }
      final sdp = await link.offer(video: video);
      if (!_live(generation)) {
        return;
      }
      final id = await signaling.offer(chat, video: video, sdp: sdp);
      if (!_live(generation)) {
        // Hung up while the call was on its way.
        unawaited(_quietly(signaling.end(chat, id, CallEnding.hangup)));
        return;
      }
      _set(_state!.copyWith(id: id));
      _expire(noAnswer, generation, () {
        unawaited(_quietly(signaling.end(chat, id, CallEnding.missed)));
        _finish(CallEndNote.noAnswer);
      });
    } on Object {
      if (_live(generation)) {
        _finish(CallEndNote.failed);
      }
    }
  }

  /// Whether the call of [generation] is still ringing or going on.
  bool _live(int generation) => generation == _generation && busy;

  /// Picks up the call ringing here.
  Future<void> accept() async {
    final current = _state;
    final offer = _offer;
    if (current == null ||
        current.phase != CallPhase.incoming ||
        offer == null) {
      return;
    }
    final generation = _generation;
    ringer.stop();
    _set(current.copyWith(phase: CallPhase.connecting));
    _expire(connectFor, generation, () => _fail(generation));
    try {
      final link = await _connect(generation);
      if (link == null) {
        return;
      }
      final sdp = await link.answer(offer, video: current.video);
      if (!_live(generation)) {
        return;
      }
      await signaling.answer(current.chat, current.id!, sdp);
    } on Object {
      _fail(generation);
    }
  }

  /// Turns down the call ringing here.
  Future<void> decline() async {
    final current = _state;
    if (current == null || current.phase != CallPhase.incoming) {
      return;
    }
    _finish(null);
    await _quietly(
      signaling.end(current.chat, current.id!, CallEnding.declined),
    );
  }

  /// Ends the call: hangs up, gives up calling, or turns it down.
  Future<void> hangUp() async {
    final current = _state;
    if (current == null || current.phase == CallPhase.ended) {
      return;
    }
    if (current.phase == CallPhase.incoming) {
      return await decline();
    }
    final id = current.id;
    final talked =
        current.phase == CallPhase.connecting ||
        current.phase == CallPhase.active;
    final duration = _durationMs();
    _finish(talked ? CallEndNote.ended : null);
    if (id != null) {
      await _quietly(
        signaling.end(
          current.chat,
          id,
          CallEnding.hangup,
          durationMs: duration,
        ),
      );
    }
  }

  /// Turns the microphone off or on.
  Future<void> setMuted({required bool muted}) async {
    final current = _state;
    if (current == null || !busy) {
      return;
    }
    _set(current.copyWith(muted: muted));
    await _quietly(_link?.setMuted(muted: muted));
  }

  /// Sends the sound to the loudspeaker or back to the earpiece.
  Future<void> setSpeaker({required bool on}) async {
    final current = _state;
    if (current == null || !busy) {
      return;
    }
    _set(current.copyWith(speaker: on));
    await _quietly(_link?.setSpeaker(on: on));
  }

  /// Turns the camera off or on in a video call.
  Future<void> setCamera({required bool on}) async {
    final current = _state;
    if (current == null || !busy) {
      return;
    }
    _set(current.copyWith(cameraOff: !on));
    await _quietly(_link?.setCamera(on: on));
  }

  /// Switches between the front and back camera.
  Future<void> switchCamera() => _quietly(_link?.switchCamera());

  /// Acts on a call message from the chat.
  void handle(CallSignal signal) {
    switch (signal.kind) {
      case CallSignalKind.offer:
        _offered(signal);
      case CallSignalKind.ringing:
        if (_isCurrent(signal) && _state!.phase == CallPhase.calling) {
          _set(_state!.copyWith(phase: CallPhase.ringing));
          ringer.ringBack();
        }
      case CallSignalKind.answer:
        unawaited(_answered(signal));
      case CallSignalKind.end:
        _ended(signal);
    }
  }

  void _offered(CallSignal signal) {
    if (signal.own) {
      return;
    }
    final current = _state;
    if (current != null && busy) {
      if (_crossed(current, signal) && _less(signal.call, current.id!)) {
        // Both called at once, and theirs wins: withdraw ours and pick
        // theirs up, since both people want to talk (`calls.md`, rule 7).
        unawaited(
          _quietly(signaling.end(current.chat, current.id!, CallEnding.hangup)),
        );
        _ring(signal);
        unawaited(accept());
      }
      // Otherwise busy in another call, or ours wins: no ringing.
      return;
    }
    if (signal.ring && signal.sdp != null) {
      _ring(signal);
      ringer.ring();
      onRing?.call(_state!);
    }
  }

  /// Makes [signal]'s call the one ringing here.
  void _ring(CallSignal signal) {
    final generation = _begin(
      CallState(
        chat: signal.chat,
        peer: signal.peer,
        video: signal.video,
        outgoing: false,
        phase: CallPhase.incoming,
        id: signal.call,
      ),
    );
    _offer = signal.sdp;
    // The caller gives up first and says so; this is in case that never
    // arrives.
    _expire(ringFor, generation, () => _finish(null));
  }

  Future<void> _answered(CallSignal signal) async {
    if (!_isCurrent(signal)) {
      return;
    }
    final current = _state!;
    if (!current.outgoing) {
      if (signal.own && current.phase == CallPhase.incoming) {
        // Picked up on another device.
        _finish(null);
      } else if (signal.own) {
        _rival = true;
      }
      return;
    }
    final waiting =
        current.phase == CallPhase.calling ||
        current.phase == CallPhase.ringing;
    final answer = signal.sdp;
    if (signal.own || !waiting || answer == null) {
      return;
    }
    final generation = _generation;
    ringer.stop();
    _set(current.copyWith(phase: CallPhase.connecting));
    _expire(connectFor, generation, () => _fail(generation));
    try {
      await _link?.accept(answer);
    } on Object {
      _fail(generation);
    }
  }

  void _ended(CallSignal signal) {
    if (!_isCurrent(signal)) {
      return;
    }
    final current = _state!;
    if (current.phase == CallPhase.incoming) {
      // The caller gave up, or another of our devices said no: the chat
      // shows the missed call.
      _finish(null);
      return;
    }
    _finish(switch (signal.ending) {
      CallEnding.declined => CallEndNote.declined,
      CallEnding.missed => CallEndNote.noAnswer,
      CallEnding.busy => CallEndNote.busy,
      CallEnding.failed => CallEndNote.failed,
      _ => CallEndNote.ended,
    });
  }

  /// The media link for this call, with the servers the service names;
  /// `null` if the call ended meanwhile.
  Future<MediaLink?> _connect(int generation) async {
    List<IceServerView> servers;
    try {
      servers = await signaling.servers();
    } on Object {
      servers = _fallbackServers;
    }
    if (!_live(generation)) {
      return null;
    }
    final link = links(servers);
    _link = link;
    _media = link.states.listen((state) => _mediaChanged(state, generation));
    void follow() {
      remoteVideo.value = link.remoteVideo.value;
      localVideo.value = link.localVideo.value;
    }

    link.remoteVideo.addListener(follow);
    link.localVideo.addListener(follow);
    _unfollow = () {
      link.remoteVideo.removeListener(follow);
      link.localVideo.removeListener(follow);
    };
    follow();
    return link;
  }

  void _mediaChanged(MediaState media, int generation) {
    final current = _state;
    if (generation != _generation || current == null || !busy) {
      return;
    }
    switch (media) {
      case MediaState.connected:
        _lost?.cancel();
        _lost = null;
        if (current.phase == CallPhase.connecting) {
          _deadline?.cancel();
          _rival = false;
          _set(
            current.copyWith(phase: CallPhase.active, connectedAt: _clock()),
          );
        }
      case MediaState.interrupted:
        _lost ??= Timer(lostFor, () => _fail(generation));
      case MediaState.failed:
        _fail(generation);
      case MediaState.connecting:
        break;
    }
  }

  /// The connection could not be made or was lost.
  void _fail(int generation) {
    final current = _state;
    if (generation != _generation || current == null || !busy) {
      return;
    }
    final id = current.id;
    final duration = _durationMs();
    final rival = _rival;
    // Another device of ours took the call: nothing to report.
    _finish(rival ? null : CallEndNote.failed);
    if (id != null && !rival) {
      unawaited(
        _quietly(
          signaling.end(
            current.chat,
            id,
            CallEnding.failed,
            durationMs: duration,
          ),
        ),
      );
    }
  }

  /// Starts a new call as [state]; returns its generation.
  int _begin(CallState state) {
    _release();
    _set(state);
    return ++_generation;
  }

  /// Ends the call here. With a [note], the screen says how for
  /// [endedFor]; without, it goes away at once.
  void _finish(CallEndNote? note) {
    final current = _state;
    _release();
    if (note == null || current == null) {
      _set(null);
      return;
    }
    _set(current.copyWith(phase: CallPhase.ended, note: note));
    final generation = _generation;
    _deadline = Timer(endedFor, () {
      if (generation == _generation && _state?.phase == CallPhase.ended) {
        _set(null);
      }
    });
  }

  /// Lets go of everything the current call holds.
  void _release() {
    ringer.stop();
    _deadline?.cancel();
    _deadline = null;
    _lost?.cancel();
    _lost = null;
    unawaited(_media?.cancel());
    _media = null;
    _unfollow?.call();
    _unfollow = null;
    remoteVideo.value = null;
    localVideo.value = null;
    final link = _link;
    _link = null;
    if (link != null) {
      unawaited(_quietly(link.close()));
    }
    _offer = null;
    _rival = false;
  }

  void _expire(Duration after, int generation, void Function() then) {
    _deadline?.cancel();
    _deadline = Timer(after, () {
      if (generation == _generation && busy) {
        then();
      }
    });
  }

  bool _isCurrent(CallSignal signal) {
    final current = _state;
    return current != null &&
        busy &&
        current.id == signal.call &&
        current.chat == signal.chat;
  }

  /// Whether [signal] is the other person calling while this device is
  /// still calling them.
  bool _crossed(CallState current, CallSignal signal) =>
      current.outgoing &&
      current.id != null &&
      current.chat == signal.chat &&
      (current.phase == CallPhase.calling ||
          current.phase == CallPhase.ringing);

  int? _durationMs() {
    final since = _state?.connectedAt;
    if (since == null) {
      return null;
    }
    return _clock().difference(since).inMilliseconds.clamp(0, 0xFFFFFFFF);
  }

  void _set(CallState? state) {
    _state = state;
    notifyListeners();
  }

  @override
  void dispose() {
    _release();
    _generation++;
    remoteVideo.dispose();
    localVideo.dispose();
    super.dispose();
  }
}

/// Whether call id [a] sorts before [b], byte by byte.
bool _less(String a, String b) {
  final left = base64Url.decode(base64Url.normalize(a));
  final right = base64Url.decode(base64Url.normalize(b));
  for (var i = 0; i < min(left.length, right.length); i++) {
    if (left[i] != right[i]) {
      return left[i] < right[i];
    }
  }
  return left.length < right.length;
}

/// Waits for [work], ignoring any failure: a message that cannot be sent
/// now goes out with the outbox later, and a closing link is gone anyway.
Future<void> _quietly(Future<void>? work) async {
  try {
    await work;
  } on Object {
    // Nothing more to do.
  }
}
