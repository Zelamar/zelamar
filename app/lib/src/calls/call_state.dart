/// Where a call is (ADR-0019).
enum CallPhase {
  /// This device is calling; the other side has not got the call yet.
  calling,

  /// The other person's device is ringing.
  ringing,

  /// Someone is calling this device.
  incoming,

  /// Picked up; the two devices are connecting.
  connecting,

  /// The two people are talking.
  active,

  /// Over; the screen says how for a moment, then goes away.
  ended,
}

/// How a call ended, as the call screen says it.
enum CallEndNote {
  /// Someone hung up.
  ended,

  /// The person called said no.
  declined,

  /// Nobody picked up in time.
  noAnswer,

  /// The person called is in another call.
  busy,

  /// The devices could not connect, or lost each other.
  failed,
}

/// A snapshot of the call ringing or going on.
class CallState {
  const new({
    required this.chat,
    required this.peer,
    required this.video,
    required this.outgoing,
    required this.phase,
    this.id,
    this.connectedAt,
    this.muted = false,
    this.speaker = false,
    this.cameraOff = false,
    this.note,
  });

  /// The chat the call belongs to.
  final String chat;

  /// The call's id; `null` until an outgoing call has been sent.
  final String? id;

  /// The other person: `@bob`.
  final String peer;

  /// Whether the call started with video.
  final bool video;

  /// Whether this device made the call.
  final bool outgoing;

  final CallPhase phase;

  /// When the two devices connected, by this device's clock.
  final DateTime? connectedAt;

  /// Whether the microphone is off.
  final bool muted;

  /// Whether sound goes to the loudspeaker (phones).
  final bool speaker;

  /// Whether this device's camera is off in a video call.
  final bool cameraOff;

  /// How it ended, once [phase] is [CallPhase.ended].
  final CallEndNote? note;

  CallState copyWith({
    String? id,
    CallPhase? phase,
    DateTime? connectedAt,
    bool? muted,
    bool? speaker,
    bool? cameraOff,
    CallEndNote? note,
  }) => CallState(
    chat: chat,
    peer: peer,
    video: video,
    outgoing: outgoing,
    id: id ?? this.id,
    phase: phase ?? this.phase,
    connectedAt: connectedAt ?? this.connectedAt,
    muted: muted ?? this.muted,
    speaker: speaker ?? this.speaker,
    cameraOff: cameraOff ?? this.cameraOff,
    note: note ?? this.note,
  );
}
