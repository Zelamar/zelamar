import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_webrtc/flutter_webrtc.dart';
import 'package:zelamar/src/calls/media_link.dart';
import 'package:zelamar/src/rust/api/session.dart';

/// A call's media over WebRTC (ADR-0019): one peer connection, the
/// microphone (and camera), DTLS-SRTP straight to the other device.
class WebRtcLink implements MediaLink {
  new(this.servers, {this.gathering = const Duration(seconds: 3)});

  /// The servers that help the connection: STUN, perhaps a TURN relay.
  final List<IceServerView> servers;

  /// How long to gather candidates before the description goes out
  /// without the slowest ones.
  final Duration gathering;

  RTCPeerConnection? _connection;
  MediaStream? _local;
  RTCVideoRenderer? _localRenderer;
  RTCVideoRenderer? _remoteRenderer;
  final _remoteVideo = ValueNotifier<Widget?>(null);
  final _localVideo = ValueNotifier<Widget?>(null);
  Widget? _localView;
  final _gathered = Completer<void>();
  final _states = StreamController<MediaState>.broadcast();
  bool _closed = false;

  @override
  Stream<MediaState> get states => _states.stream;

  @override
  ValueListenable<Widget?> get remoteVideo => _remoteVideo;

  @override
  ValueListenable<Widget?> get localVideo => _localVideo;

  @override
  Future<String> offer({required bool video}) async {
    final connection = await _open(video: video);
    final offer = await connection.createOffer({});
    await connection.setLocalDescription(offer);
    return await _description(connection);
  }

  @override
  Future<String> answer(String offer, {required bool video}) async {
    final connection = await _open(video: video);
    await connection.setRemoteDescription(
      RTCSessionDescription(offer, 'offer'),
    );
    final answer = await connection.createAnswer({});
    await connection.setLocalDescription(answer);
    return await _description(connection);
  }

  @override
  Future<void> accept(String answer) async {
    final connection = _connection;
    if (connection == null) {
      throw StateError('no offer was made');
    }
    await connection.setRemoteDescription(
      RTCSessionDescription(answer, 'answer'),
    );
  }

  @override
  Future<void> setMuted({required bool muted}) async {
    for (final track in _local?.getAudioTracks() ?? <MediaStreamTrack>[]) {
      track.enabled = !muted;
    }
  }

  @override
  Future<void> setSpeaker({required bool on}) async {
    if (Platform.isAndroid || Platform.isIOS) {
      await Helper.setSpeakerphoneOn(on);
    }
  }

  @override
  Future<void> setCamera({required bool on}) async {
    for (final track in _local?.getVideoTracks() ?? <MediaStreamTrack>[]) {
      track.enabled = on;
    }
    _localVideo.value = on ? _localView : null;
  }

  @override
  Future<void> switchCamera() async {
    final track = _local?.getVideoTracks().firstOrNull;
    if (track != null && (Platform.isAndroid || Platform.isIOS)) {
      await Helper.switchCamera(track);
    }
  }

  @override
  Future<void> close() async {
    if (_closed) {
      return;
    }
    _closed = true;
    _remoteVideo.value = null;
    _localVideo.value = null;
    final local = _local;
    _local = null;
    for (final track in local?.getTracks() ?? <MediaStreamTrack>[]) {
      await track.stop();
    }
    await local?.dispose();
    for (final renderer in [_localRenderer, _remoteRenderer]) {
      renderer?.srcObject = null;
      await renderer?.dispose();
    }
    final connection = _connection;
    _connection = null;
    await connection?.close();
    await connection?.dispose();
    await _states.close();
    _remoteVideo.dispose();
    _localVideo.dispose();
  }

  /// The peer connection, with this device's microphone (and camera) on
  /// it.
  Future<RTCPeerConnection> _open({required bool video}) async {
    if (Platform.isAndroid) {
      // Voice-call audio: the earpiece, echo cancellation, call volume.
      await Helper.setAndroidAudioConfiguration(
        AndroidAudioConfiguration.communication,
      );
    }
    final connection = await createPeerConnection({
      'iceServers': [
        for (final server in servers)
          {
            'urls': server.urls,
            if (server.username != null) 'username': server.username,
            if (server.credential != null) 'credential': server.credential,
          },
      ],
      'sdpSemantics': 'unified-plan',
      'bundlePolicy': 'max-bundle',
      'rtcpMuxPolicy': 'require',
    });
    _connection = connection;
    connection
      ..onIceGatheringState = (state) {
        if (state == RTCIceGatheringState.RTCIceGatheringStateComplete &&
            !_gathered.isCompleted) {
          _gathered.complete();
        }
      }
      ..onConnectionState = (state) {
        final media = switch (state) {
          RTCPeerConnectionState.RTCPeerConnectionStateConnected =>
            MediaState.connected,
          RTCPeerConnectionState.RTCPeerConnectionStateDisconnected =>
            MediaState.interrupted,
          RTCPeerConnectionState.RTCPeerConnectionStateFailed =>
            MediaState.failed,
          RTCPeerConnectionState.RTCPeerConnectionStateClosed => null,
          _ => MediaState.connecting,
        };
        if (media != null && !_closed) {
          _states.add(media);
        }
      };
    if (video) {
      final remote = RTCVideoRenderer();
      await remote.initialize();
      _remoteRenderer = remote;
      connection.onTrack = (event) {
        if (event.track.kind != 'video' || _closed) {
          return;
        }
        remote.srcObject = event.streams.firstOrNull;
        _remoteVideo.value = RTCVideoView(
          remote,
          objectFit: RTCVideoViewObjectFit.RTCVideoViewObjectFitCover,
        );
      };
    }
    final local = await _capture(video: video);
    _local = local;
    for (final track in local?.getTracks() ?? <MediaStreamTrack>[]) {
      await connection.addTrack(track, local!);
    }
    // Without a microphone or camera, still hear and see the other person.
    if (local == null || local.getAudioTracks().isEmpty) {
      await _receiveOnly(connection, RTCRtpMediaType.RTCRtpMediaTypeAudio);
    }
    if (video && (local == null || local.getVideoTracks().isEmpty)) {
      await _receiveOnly(connection, RTCRtpMediaType.RTCRtpMediaTypeVideo);
    }
    if (video && local != null && local.getVideoTracks().isNotEmpty) {
      final renderer = RTCVideoRenderer();
      await renderer.initialize();
      renderer.srcObject = local;
      _localRenderer = renderer;
      _localView = RTCVideoView(
        renderer,
        mirror: true,
        objectFit: RTCVideoViewObjectFit.RTCVideoViewObjectFitCover,
      );
      _localVideo.value = _localView;
    }
    return connection;
  }

  /// The microphone, and the camera for a video call; whatever of them is
  /// there and allowed.
  Future<MediaStream?> _capture({required bool video}) async {
    final attempts = [
      if (video)
        {
          'audio': true,
          'video': {'facingMode': 'user'},
        },
      {'audio': true, 'video': false},
    ];
    for (final constraints in attempts) {
      try {
        return await navigator.mediaDevices.getUserMedia(constraints);
      } on Object {
        // Try with less.
      }
    }
    return null;
  }

  Future<void> _receiveOnly(
    RTCPeerConnection connection,
    RTCRtpMediaType kind,
  ) => connection.addTransceiver(
    kind: kind,
    init: RTCRtpTransceiverInit(direction: TransceiverDirection.RecvOnly),
  );

  /// This side's description once every candidate is in it, or after
  /// [gathering] with those found so far.
  Future<String> _description(RTCPeerConnection connection) async {
    await _gathered.future.timeout(gathering, onTimeout: () {});
    final sdp = (await connection.getLocalDescription())?.sdp;
    if (sdp == null || sdp.isEmpty) {
      throw StateError('no local description');
    }
    return sdp;
  }
}
