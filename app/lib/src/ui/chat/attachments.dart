import 'dart:async';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';
import 'dart:ui';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import 'package:share_plus/share_plus.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/words.dart';

/// Fetches and opens a message's file.
typedef FileLoader = Future<Uint8List> Function();

/// A photo in a bubble: the tiny preview at once, blurred, then the photo
/// itself; a tap shows it full screen.
class ImageAttachment extends StatefulWidget {
  const new({required this.file, required this.load, super.key});

  final FileView file;
  final FileLoader load;

  @override
  State<ImageAttachment> createState() => _ImageAttachmentState();
}

class _ImageAttachmentState extends State<ImageAttachment> {
  Uint8List? _full;
  bool _failed = false;

  @override
  void initState() {
    super.initState();
    unawaited(_fetch());
  }

  Future<void> _fetch() async {
    try {
      final bytes = await widget.load();
      if (mounted) {
        setState(() => _full = bytes);
      }
    } on MessengerFailure {
      if (mounted) {
        setState(() => _failed = true);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final file = widget.file;
    final width = file.width ?? 4;
    final height = file.height ?? 3;
    final ratio = (width / height).clamp(0.5, 2.0);
    final full = _full;
    final thumb = file.thumb;
    final picture = full != null
        ? Image.memory(full, fit: BoxFit.cover, gaplessPlayback: true)
        : thumb != null
        ? ImageFiltered(
            imageFilter: ImageFilter.blur(sigmaX: 6, sigmaY: 6),
            child: Image.memory(thumb, fit: BoxFit.cover),
          )
        : ColoredBox(color: context.colors.surfaceSunken);
    return GestureDetector(
      key: const Key('chat.image'),
      onTap: full == null
          ? (_failed ? _retry : null)
          : () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) =>
                    ImageViewer(bytes: full, name: file.name, mime: file.mime),
              ),
            ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(ZRadius.field),
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxWidth: min(MediaQuery.sizeOf(context).width * 0.66, 360),
          ),
          child: AspectRatio(
            aspectRatio: ratio,
            child: Stack(
              fit: StackFit.expand,
              children: [
                picture,
                if (full == null)
                  Center(
                    child: _failed
                        ? const Icon(Icons.refresh_rounded, color: Colors.white)
                        : const SizedBox.square(
                            dimension: 22,
                            child: CircularProgressIndicator(
                              strokeWidth: 2,
                              color: Colors.white,
                            ),
                          ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  void _retry() {
    setState(() => _failed = false);
    unawaited(_fetch());
  }
}

/// A photo full screen, to zoom and save.
class ImageViewer extends StatelessWidget {
  const new({
    required this.bytes,
    required this.name,
    required this.mime,
    super.key,
  });

  final Uint8List bytes;
  final String name;
  final String mime;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        backgroundColor: Colors.black,
        foregroundColor: Colors.white,
        actions: [
          IconButton(
            key: const Key('viewer.save'),
            tooltip: 'Save',
            icon: const Icon(Icons.download_rounded),
            onPressed: () =>
                unawaited(saveFile(context, name, bytes, mime: mime)),
          ),
        ],
      ),
      body: InteractiveViewer(
        maxScale: 6,
        child: Center(child: Image.memory(bytes)),
      ),
    );
  }
}

/// Any other file in a bubble: its name and size; a tap fetches it and
/// asks where to save it.
class FileAttachment extends StatefulWidget {
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
  State<FileAttachment> createState() => _FileAttachmentState();
}

class _FileAttachmentState extends State<FileAttachment> {
  bool _busy = false;

  Future<void> _open() async {
    setState(() => _busy = true);
    try {
      final bytes = await widget.load();
      if (mounted) {
        await saveFile(
          context,
          widget.file.name,
          bytes,
          mime: widget.file.mime,
        );
      }
    } on MessengerFailure catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(describeFailure(failure.code))));
      }
    } finally {
      if (mounted) {
        setState(() => _busy = false);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    final ink = widget.outgoing ? colors.onAccent : colors.textPrimary;
    final quiet = widget.outgoing
        ? colors.bubbleOutgoingMeta
        : colors.textTertiary;
    return InkWell(
      key: const Key('chat.file'),
      onTap: _busy ? null : _open,
      borderRadius: BorderRadius.circular(ZRadius.field),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: ZSpace.xs),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox.square(
              dimension: 36,
              child: _busy
                  ? Padding(
                      padding: const EdgeInsets.all(8),
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        color: ink,
                      ),
                    )
                  : Icon(Icons.insert_drive_file_outlined, color: ink),
            ),
            const SizedBox(width: ZSpace.sm),
            Flexible(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    widget.file.name,
                    overflow: TextOverflow.ellipsis,
                    style: text.bodyMedium?.copyWith(color: ink),
                  ),
                  Text(
                    describeSize(widget.file.size.toInt()),
                    style: text.labelSmall?.copyWith(color: quiet),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Asks where to save [bytes] as [name], and saves them there. Android has
/// no save dialog for apps like this one: its share sheet saves to Files or
/// opens the file in another app instead.
Future<void> saveFile(
  BuildContext context,
  String name,
  Uint8List bytes, {
  String? mime,
}) async {
  final messenger = ScaffoldMessenger.of(context);
  try {
    if (Platform.isAndroid) {
      await SharePlus.instance.share(
        ShareParams(
          files: [XFile.fromData(bytes, name: name, mimeType: mime)],
          fileNameOverrides: [name],
        ),
      );
      return;
    }
    final location = await getSaveLocation(suggestedName: name);
    if (location == null) {
      return;
    }
    await XFile.fromData(bytes, name: name).saveTo(location.path);
    messenger.showSnackBar(const SnackBar(content: Text('Saved')));
  } on Object {
    messenger.showSnackBar(
      const SnackBar(content: Text('Zelamar couldn’t save the file there.')),
    );
  }
}
