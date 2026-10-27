import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:zelamar/src/calls/call_state.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/avatar.dart';
import 'package:zelamar/src/ui/words.dart';

/// Covers everything while a call rings or goes on (ADR-0019): who, where
/// the call is, and the few things one can do. Shows nothing otherwise.
class CallOverlay extends StatelessWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: messenger,
      builder: (context, _) {
        final call = messenger.call;
        return AnimatedSwitcher(
          duration: ZMotion.slow,
          switchInCurve: ZMotion.enter,
          switchOutCurve: ZMotion.exit,
          child: call == null
              ? const SizedBox.shrink()
              : CallScreen(
                  key: ValueKey('call.${call.chat}.${call.outgoing}'),
                  messenger: messenger,
                  call: call,
                ),
        );
      },
    );
  }
}

/// One call, full screen.
class CallScreen extends StatefulWidget {
  const new({required this.messenger, required this.call, super.key});

  final Messenger messenger;
  final CallState call;

  @override
  State<CallScreen> createState() => _CallScreenState();
}

class _CallScreenState extends State<CallScreen> {
  /// Moves the clock on while the people talk.
  Timer? _tick;

  @override
  void initState() {
    super.initState();
    _tick = Timer.periodic(const Duration(seconds: 1), (_) {
      if (mounted && widget.call.phase == CallPhase.active) {
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
    if (!widget.call.video) {
      return _stage(context, null);
    }
    return ValueListenableBuilder<Widget?>(
      valueListenable: widget.messenger.remoteVideo,
      builder: (context, picture, _) => picture == null
          ? _stage(context, null)
          // Over a picture, the espresso palette: light words on dark.
          : Theme(
              data: zelamarTheme(Brightness.dark),
              child: Builder(builder: (context) => _stage(context, picture)),
            ),
    );
  }

  /// The call, over the other person's [picture] in a video call.
  Widget _stage(BuildContext context, Widget? picture) {
    final colors = context.colors;
    final text = context.text;
    final call = widget.call;
    final ringing =
        call.phase == CallPhase.calling ||
        call.phase == CallPhase.ringing ||
        call.phase == CallPhase.incoming;
    final onPicture = picture != null;
    return Material(
      color: colors.background,
      child: Stack(
        fit: StackFit.expand,
        children: [
          if (picture != null) ...[picture, const _Shade()],
          SafeArea(
            child: Center(
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 480),
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: ZSpace.gutter,
                  ),
                  child: Column(
                    crossAxisAlignment: onPicture
                        ? CrossAxisAlignment.start
                        : CrossAxisAlignment.center,
                    children: [
                      if (onPicture)
                        const SizedBox(height: ZSpace.xl)
                      else ...[
                        const Spacer(flex: 2),
                        _Halo(
                          active: ringing,
                          child: Avatar(call.peer, size: 112),
                        ),
                        const SizedBox(height: ZSpace.xl),
                      ],
                      Padding(
                        // Clear of this device's own picture, top right.
                        padding: EdgeInsets.only(
                          right: onPicture ? _previewWidth + ZSpace.lg : 0,
                        ),
                        child: Column(
                          crossAxisAlignment: onPicture
                              ? CrossAxisAlignment.start
                              : CrossAxisAlignment.center,
                          children: [
                            Text(
                              call.peer,
                              key: const Key('call.peer'),
                              style: onPicture
                                  ? text.headlineSmall
                                  : text.displaySmall,
                              textAlign: TextAlign.center,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                            ),
                            const SizedBox(height: ZSpace.sm),
                            Text(
                              _status(call),
                              key: const Key('call.status'),
                              style: text.bodyLarge?.copyWith(
                                color: call.phase == CallPhase.ended
                                    ? colors.textTertiary
                                    : colors.textSecondary,
                                fontFeatures: const [
                                  FontFeature.tabularFigures(),
                                ],
                              ),
                            ),
                          ],
                        ),
                      ),
                      const Spacer(flex: 3),
                      Center(
                        child: AnimatedOpacity(
                          opacity: call.phase == CallPhase.ended ? 0 : 1,
                          duration: ZMotion.base,
                          child: call.phase == CallPhase.incoming
                              ? _Answering(widget.messenger, video: call.video)
                              : _Talking(widget.messenger, call),
                        ),
                      ),
                      const SizedBox(height: ZSpace.xxxl),
                    ],
                  ),
                ),
              ),
            ),
          ),
          if (call.video && call.phase != CallPhase.incoming)
            Positioned(
              top: 0,
              right: 0,
              child: SafeArea(
                child: Padding(
                  padding: const EdgeInsets.all(ZSpace.lg),
                  child: _Preview(widget.messenger.localVideo),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

const double _previewWidth = 104;

/// This device's own picture, small, while its camera is on.
class _Preview extends StatelessWidget {
  const new(this.picture);

  final ValueListenable<Widget?> picture;

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<Widget?>(
      valueListenable: picture,
      builder: (context, picture, _) => picture == null
          ? const SizedBox.shrink()
          : Container(
              key: const Key('call.preview'),
              width: _previewWidth,
              height: _previewWidth * 4 / 3,
              clipBehavior: Clip.antiAlias,
              decoration: BoxDecoration(
                color: context.colors.surfaceSunken,
                borderRadius: BorderRadius.circular(ZRadius.field),
                border: Border.all(color: context.colors.hairline),
              ),
              child: picture,
            ),
    );
  }
}

/// Darkens the top and bottom of the picture a little, so the name and the
/// buttons read on any picture.
class _Shade extends StatelessWidget {
  const new();

  @override
  Widget build(BuildContext context) {
    final shade = context.colors.background;
    return IgnorePointer(
      child: DecoratedBox(
        decoration: BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: [
              shade.withValues(alpha: 0.6),
              shade.withValues(alpha: 0),
              shade.withValues(alpha: 0),
              shade.withValues(alpha: 0.7),
            ],
            stops: const [0, 0.25, 0.6, 1],
          ),
        ),
      ),
    );
  }
}

/// Where the call is, in a word or two; while talking, for how long.
String _status(CallState call) => switch (call.phase) {
  CallPhase.calling => 'Calling…',
  CallPhase.ringing => 'Ringing…',
  CallPhase.incoming => call.video ? 'Incoming video call' : 'Incoming call',
  CallPhase.connecting => 'Connecting…',
  CallPhase.active => describeDuration(
    DateTime.now().difference(call.connectedAt ?? DateTime.now()),
  ),
  CallPhase.ended => switch (call.note) {
    CallEndNote.declined => 'Declined',
    CallEndNote.noAnswer => 'No answer',
    CallEndNote.busy => 'Busy',
    CallEndNote.failed => 'Couldn’t connect',
    _ => 'Call ended',
  },
};

/// Decline or pick up.
class _Answering extends StatelessWidget {
  const new(this.messenger, {required this.video});

  final Messenger messenger;
  final bool video;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Row(
      mainAxisAlignment: MainAxisAlignment.spaceEvenly,
      children: [
        _RoundButton(
          key: const Key('call.decline'),
          icon: Icons.call_end,
          label: 'Decline',
          color: colors.danger,
          onColor: colors.onAccent,
          onPressed: () => unawaited(messenger.declineCall()),
        ),
        _RoundButton(
          key: const Key('call.accept'),
          icon: video ? Icons.videocam : Icons.call,
          label: 'Accept',
          color: colors.online,
          onColor: colors.onAccent,
          onPressed: () => unawaited(messenger.acceptCall()),
        ),
      ],
    );
  }
}

/// Microphone, loudspeaker (on phones), hang up.
class _Talking extends StatelessWidget {
  const new(this.messenger, this.call);

  final Messenger messenger;
  final CallState call;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final phone = Platform.isAndroid || Platform.isIOS;
    return Row(
      mainAxisAlignment: MainAxisAlignment.spaceEvenly,
      children: [
        _RoundButton(
          key: const Key('call.mute'),
          icon: call.muted ? Icons.mic_off : Icons.mic_none,
          label: call.muted ? 'Unmute' : 'Mute',
          color: call.muted ? colors.accent : colors.surfaceSunken,
          onColor: call.muted ? colors.onAccent : colors.textPrimary,
          onPressed: () => unawaited(messenger.setMuted(muted: !call.muted)),
        ),
        if (call.video)
          _RoundButton(
            key: const Key('call.camera'),
            icon: call.cameraOff ? Icons.videocam_off : Icons.videocam_outlined,
            label: 'Camera',
            color: call.cameraOff ? colors.accent : colors.surfaceSunken,
            onColor: call.cameraOff ? colors.onAccent : colors.textPrimary,
            onPressed: () => unawaited(messenger.setCamera(on: call.cameraOff)),
          ),
        if (call.video && phone)
          _RoundButton(
            key: const Key('call.flip'),
            icon: Icons.cameraswitch_outlined,
            label: 'Flip',
            color: colors.surfaceSunken,
            onColor: colors.textPrimary,
            onPressed: () => unawaited(messenger.switchCamera()),
          ),
        if (phone && !call.video)
          _RoundButton(
            key: const Key('call.speaker'),
            icon: call.speaker ? Icons.volume_up : Icons.volume_down_outlined,
            label: 'Speaker',
            color: call.speaker ? colors.accent : colors.surfaceSunken,
            onColor: call.speaker ? colors.onAccent : colors.textPrimary,
            onPressed: () => unawaited(messenger.setSpeaker(on: !call.speaker)),
          ),
        _RoundButton(
          key: const Key('call.end'),
          icon: Icons.call_end,
          label: 'End',
          color: colors.danger,
          onColor: colors.onAccent,
          onPressed: () => unawaited(messenger.hangUp()),
        ),
      ],
    );
  }
}

class _RoundButton extends StatelessWidget {
  const new({
    required this.icon,
    required this.label,
    required this.color,
    required this.onColor,
    required this.onPressed,
    super.key,
  });

  final IconData icon;
  final String label;
  final Color color;
  final Color onColor;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Semantics(
          button: true,
          label: label,
          child: Material(
            color: color,
            shape: const CircleBorder(),
            child: InkWell(
              customBorder: const CircleBorder(),
              onTap: onPressed,
              child: SizedBox(
                width: 68,
                height: 68,
                child: Icon(icon, color: onColor, size: 30),
              ),
            ),
          ),
        ),
        const SizedBox(height: ZSpace.sm),
        ExcludeSemantics(
          child: Text(
            label,
            style: context.text.bodySmall?.copyWith(
              color: context.colors.textSecondary,
            ),
          ),
        ),
      ],
    );
  }
}

/// A soft ring of light that breathes around the avatar while a call
/// rings. Calm: it fades, nothing bounces.
class _Halo extends StatefulWidget {
  const new({required this.active, required this.child});

  final bool active;
  final Widget child;

  @override
  State<_Halo> createState() => _HaloState();
}

class _HaloState extends State<_Halo> with SingleTickerProviderStateMixin {
  late final AnimationController _breath = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1600),
  );

  @override
  void initState() {
    super.initState();
    _sync();
  }

  @override
  void didUpdateWidget(_Halo old) {
    super.didUpdateWidget(old);
    _sync();
  }

  void _sync() {
    if (widget.active && !_breath.isAnimating) {
      _breath.repeat(reverse: true);
    } else if (!widget.active && _breath.isAnimating) {
      _breath
        ..stop()
        ..value = 0;
    }
  }

  @override
  void dispose() {
    _breath.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final accent = context.colors.accent;
    return AnimatedBuilder(
      animation: _breath,
      builder: (context, child) {
        final t = Curves.easeInOut.transform(_breath.value);
        return Container(
          padding: const EdgeInsets.all(ZSpace.md),
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            border: Border.all(
              color: accent.withValues(
                alpha: widget.active ? 0.12 + 0.28 * t : 0,
              ),
              width: 2,
            ),
          ),
          child: child,
        );
      },
      child: widget.child,
    );
  }
}
