import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:image/image.dart' as img;

/// A file the user picked, ready to send: its bytes and what the chat shows
/// about it (ADR-0016).
class OutgoingFile {
  const new({
    required this.name,
    required this.bytes,
    required this.mime,
    this.width,
    this.height,
    this.thumb,
    this.durationMs,
  });

  final String name;
  final Uint8List bytes;
  final String mime;
  final int? width;
  final int? height;

  /// A tiny JPEG preview, for images.
  final Uint8List? thumb;
  final int? durationMs;

  bool get isImage => mime.startsWith('image/');

  /// Prepares [bytes] named [name]: images are measured, given a preview,
  /// and very large ones are scaled down to [maxSide] pixels.
  static Future<OutgoingFile> prepare(String name, Uint8List bytes) async {
    final mime = mimeFor(name);
    if (!mime.startsWith('image/') || mime == 'image/gif') {
      return OutgoingFile(name: name, bytes: bytes, mime: mime);
    }
    final prepared = await compute(_prepareImage, (name, bytes));
    return prepared ?? OutgoingFile(name: name, bytes: bytes, mime: mime);
  }
}

/// Largest file, in bytes (ADR-0016).
const int maxFileBytes = 25 * 1024 * 1024;

/// Longest side of a sent photo, in pixels.
const maxSide = 2560;

/// Longest side of a preview, in pixels.
const thumbSide = 48;

/// Largest preview, in bytes (`protocol/content.md`).
const int maxThumb = 8 * 1024;

OutgoingFile? _prepareImage((String, Uint8List) input) {
  final (name, bytes) = input;
  final decoded = img.decodeImage(bytes);
  if (decoded == null) {
    return null;
  }
  final image = img.bakeOrientation(decoded);
  final longest = max(image.width, image.height);
  var sendBytes = bytes;
  var sendName = name;
  var mime = mimeFor(name);
  var width = image.width;
  var height = image.height;
  if (longest > maxSide) {
    final scaled = image.width >= image.height
        ? img.copyResize(image, width: maxSide)
        : img.copyResize(image, height: maxSide);
    sendBytes = img.encodeJpg(scaled, quality: 85);
    width = scaled.width;
    height = scaled.height;
    mime = 'image/jpeg';
    sendName = '${_stem(name)}.jpg';
  }
  final small = image.width >= image.height
      ? img.copyResize(image, width: thumbSide)
      : img.copyResize(image, height: thumbSide);
  Uint8List? thumb;
  for (final quality in [60, 40, 20]) {
    final candidate = img.encodeJpg(small, quality: quality);
    if (candidate.length <= maxThumb) {
      thumb = candidate;
      break;
    }
  }
  return OutgoingFile(
    name: sendName,
    bytes: sendBytes,
    mime: mime,
    width: width,
    height: height,
    thumb: thumb,
  );
}

String _stem(String name) {
  final dot = name.lastIndexOf('.');
  return dot > 0 ? name.substring(0, dot) : name;
}

/// The media type for a file name, from its extension.
String mimeFor(String name) {
  final dot = name.lastIndexOf('.');
  final extension = dot < 0 ? '' : name.substring(dot + 1).toLowerCase();
  return switch (extension) {
    'jpg' || 'jpeg' => 'image/jpeg',
    'png' => 'image/png',
    'gif' => 'image/gif',
    'webp' => 'image/webp',
    'bmp' => 'image/bmp',
    'heic' => 'image/heic',
    'pdf' => 'application/pdf',
    'txt' => 'text/plain',
    'zip' => 'application/zip',
    'm4a' => 'audio/mp4',
    'mp3' => 'audio/mpeg',
    'ogg' || 'opus' => 'audio/ogg',
    'mp4' => 'video/mp4',
    'mov' => 'video/quicktime',
    'doc' => 'application/msword',
    'docx' =>
      'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    'xls' => 'application/vnd.ms-excel',
    'xlsx' =>
      'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    _ => 'application/octet-stream',
  };
}
