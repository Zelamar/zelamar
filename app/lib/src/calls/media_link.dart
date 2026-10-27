import 'package:zelamar/src/rust/api/session.dart';

/// Where the media connection between the two devices is.
enum MediaState {
  /// Finding a way to the other device.
  connecting,

  /// Sound (and picture) flow.
  connected,

  /// The way was lost; it may come back.
  interrupted,

  /// No way to the other device.
  failed,
}

/// One call's media connection: WebRTC in the app, a stand-in in tests.
///
/// Session descriptions come out complete, with every candidate in them:
/// a call takes exactly one offer and one answer (`protocol/calls.md`).
abstract interface class MediaLink {
  /// Starts the connection from this side; returns the offer.
  Future<String> offer({required bool video});

  /// Answers [offer]; returns the answer.
  Future<String> answer(String offer, {required bool video});

  /// Completes the connection with the other side's [answer].
  Future<void> accept(String answer);

  /// Where the connection is; starts at [MediaState.connecting].
  Stream<MediaState> get states;

  /// Turns the microphone off or on.
  Future<void> setMuted({required bool muted});

  /// Sends sound to the loudspeaker or back to the earpiece (phones).
  Future<void> setSpeaker({required bool on});

  /// Ends the connection and lets go of the microphone and camera.
  Future<void> close();
}

/// Makes the media link for one call, given the servers that help it
/// connect.
typedef MediaLinkFactory = MediaLink Function(List<IceServerView> servers);
