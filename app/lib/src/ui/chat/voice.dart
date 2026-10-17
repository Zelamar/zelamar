import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:audioplayers/audioplayers.dart';
import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';
import 'package:record/record.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/messenger/outgoing_file.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/chat/attachments.dart';
import 'package:zelamar/src/ui/words.dart';

/// Longest voice message.
const maxVoice = Duration(minutes: 5);

/// Records one voice message at a time into a temporary file, which it
/// deletes once the recording is read or dropped.
class VoiceRecorder {
  final _recorder = AudioRecorder();
  String? _path;
  DateTime? _started;

  /// Starts recording; `false` if the microphone is not allowed.
  Future<bool> start() async {
    if (!await _recorder.hasPermission()) {
      return false;
    }
    final directory = await getTemporaryDirectory();
    final path =
        '${directory.path}${Platform.pathSeparator}'
        'zelamar-voice-${DateTime.now().microsecondsSinceEpoch}.m4a';
    await _recorder.start(
      const RecordConfig(bitRate: 32000, sampleRate: 22050, numChannels: 1),
      path: path,
    );
    _path = path;
    _started = DateTime.now();
    return true;
  }

  /// How long the recording runs so far.
  Duration get elapsed =>
      _started == null ? Duration.zero : DateTime.now().difference(_started!);

  /// Stops and returns the voice message, or `null` if nothing was heard.
  Future<OutgoingFile?> finish() async {
    final duration = elapsed;
    final path = await _recorder.stop() ?? _path;
    _started = null;
    _path = null;
    if (path == null) {
      return null;
    }
    final file = File(path);
    try {
      final bytes = await file.readAsBytes();
      if (bytes.isEmpty || duration < const Duration(milliseconds: 500)) {
        return null;
      }
      return OutgoingFile(
        name: 'Voice message.m4a',
        bytes: bytes,
        mime: 'audio/mp4',
        durationMs: duration.inMilliseconds,
      );
    } finally {
      await _delete(file);
    }
  }

  /// Stops and throws the recording away.
  Future<void> cancel() async {
    final path = await _recorder.stop() ?? _path;
    _started = null;
    _path = null;
    if (path != null) {
      await _delete(File(path));
    }
  }

  Future<void> dispose() async {
    await cancel();
    await _recorder.dispose();
  }
}

Future<void> _delete(File file) async {
  try {
    if (file.existsSync()) {
      await file.delete();
    }
  } on FileSystemException {
    // The temporary directory gets cleaned anyway.
  }
}

/// The composer while recording: how long so far, cancel or send.
class RecordingBar extends StatefulWidget {
  const new({
    required this.recorder,
    required this.onCancel,
    required this.onSend,
    super.key,
  });

  final VoiceRecorder recorder;
  final VoidCallback onCancel;
  final VoidCallback onSend;

  @override
  State<RecordingBar> createState() => _RecordingBarState();
}

class _RecordingBarState extends State<RecordingBar> {
  Timer? _tick;

  @override
  void initState() {
    super.initState();
    _tick = Timer.periodic(const Duration(milliseconds: 250), (_) {
      if (widget.recorder.elapsed >= maxVoice) {
        widget.onSend();
      } else {
        setState(() {});
      }
    });
  }

  @override
  void dispose() {
    _tick?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Row(
      key: const Key('chat.recording'),
      children: [
        IconButton(
          key: const Key('chat.record.cancel'),
          tooltip: 'Cancel',
          onPressed: widget.onCancel,
          icon: Icon(Icons.delete_outline_rounded, color: colors.textSecondary),
        ),
        Icon(Icons.fiber_manual_record, size: 12, color: colors.danger),
        const SizedBox(width: ZSpace.sm),
        Expanded(
          child: Text(
            describeDuration(widget.recorder.elapsed),
            style: context.text.bodyLarge,
          ),
        ),
        Container(
          width: 44,
          height: 44,
          decoration: BoxDecoration(
            color: colors.accent,
            shape: BoxShape.circle,
          ),
          child: IconButton(
            key: const Key('chat.record.send'),
            tooltip: 'Send',
            onPressed: widget.onSend,
            icon: Icon(Icons.arrow_upward_rounded, color: colors.onAccent),
          ),
        ),
      ],
    );
  }
}

/// A voice message in a bubble: play or pause, and how far along.
class VoiceAttachment extends StatefulWidget {
  const new({
    required this.file,
    required this.load,
    this.outgoing = false,
    super.key,
  });

  final FileView file;
  final FileLoader load;
  final bool outgoing;

  @override
  State<VoiceAttachment> createState() => _VoiceAttachmentState();
}

class _VoiceAttachmentState extends State<VoiceAttachment> {
  AudioPlayer? _player;
  File? _file;
  bool _loading = false;
  bool _playing = false;
  Duration _position = Duration.zero;
  final _subscriptions = <StreamSubscription<Object?>>[];

  Duration get _length => Duration(milliseconds: widget.file.durationMs ?? 0);

  @override
  void dispose() {
    for (final subscription in _subscriptions) {
      unawaited(subscription.cancel());
    }
    unawaited(_player?.dispose());
    final file = _file;
    if (file != null) {
      unawaited(_delete(file));
    }
    super.dispose();
  }

  Future<void> _toggle() async {
    final player = _player;
    if (player != null) {
      if (_playing) {
        await player.pause();
      } else {
        await player.resume();
      }
      return;
    }
    setState(() => _loading = true);
    try {
      final bytes = await widget.load();
      final file = await _write(bytes);
      final created = AudioPlayer();
      _subscriptions
        ..add(
          created.onPlayerStateChanged.listen((state) {
            if (mounted) {
              setState(() => _playing = state == PlayerState.playing);
            }
          }),
        )
        ..add(
          created.onPositionChanged.listen((position) {
            if (mounted) {
              setState(() => _position = position);
            }
          }),
        )
        ..add(
          created.onPlayerComplete.listen((_) {
            if (mounted) {
              setState(() => _position = Duration.zero);
            }
          }),
        );
      _player = created;
      _file = file;
      await created.play(DeviceFileSource(file.path));
    } on MessengerFailure catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(describeFailure(failure.code))));
      }
    } finally {
      if (mounted) {
        setState(() => _loading = false);
      }
    }
  }

  /// Players read files; the voice message lives in a temporary one while
  /// this bubble is on screen.
  Future<File> _write(Uint8List bytes) async {
    final directory = await getTemporaryDirectory();
    final file = File(
      '${directory.path}${Platform.pathSeparator}'
      'zelamar-play-${DateTime.now().microsecondsSinceEpoch}.m4a',
    );
    await file.writeAsBytes(bytes, flush: true);
    return file;
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final ink = widget.outgoing ? colors.onAccent : colors.accent;
    final quiet = widget.outgoing
        ? colors.bubbleOutgoingMeta
        : colors.textTertiary;
    final length = _length.inMilliseconds;
    final progress = length == 0
        ? 0.0
        : (_position.inMilliseconds / length).clamp(0.0, 1.0);
    return SizedBox(
      width: 220,
      child: Row(
        children: [
          SizedBox.square(
            dimension: 40,
            child: _loading
                ? Padding(
                    padding: const EdgeInsets.all(10),
                    child: CircularProgressIndicator(
                      strokeWidth: 2,
                      color: ink,
                    ),
                  )
                : IconButton(
                    key: const Key('chat.voice.play'),
                    tooltip: _playing ? 'Pause' : 'Play',
                    onPressed: _toggle,
                    icon: Icon(
                      _playing ? Icons.pause_rounded : Icons.play_arrow_rounded,
                      color: ink,
                    ),
                  ),
          ),
          const SizedBox(width: ZSpace.xs),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                ClipRRect(
                  borderRadius: BorderRadius.circular(2),
                  child: LinearProgressIndicator(
                    value: progress,
                    minHeight: 3,
                    color: ink,
                    backgroundColor: quiet.withValues(alpha: 0.3),
                  ),
                ),
                const SizedBox(height: ZSpace.xs),
                Text(
                  describeDuration(
                    _playing || _position > Duration.zero ? _position : _length,
                  ),
                  key: const Key('chat.voice.time'),
                  style: context.text.labelSmall?.copyWith(color: quiet),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
