import 'dart:io';

import 'package:path_provider/path_provider.dart';

/// How old a leftover must be before a start removes it: another profile's
/// window may still be playing it.
const _settled = Duration(minutes: 10);

/// Removes decrypted copies a crash or a killed app could leave in the
/// temporary directory: voice messages being recorded or played, and files
/// handed to the share sheet. Every start sweeps them; failures are
/// ignored, as the system cleans the directory too.
Future<void> sweepLeftovers({Directory? temporary, DateTime? now}) async {
  try {
    final directory = temporary ?? await getTemporaryDirectory();
    final before = (now ?? DateTime.now()).subtract(_settled);
    void sweep(File file) {
      if (file.statSync().modified.isBefore(before)) {
        file.deleteSync();
      }
    }

    for (final entity in directory.listSync(followLinks: false)) {
      final name = entity.uri.pathSegments.lastWhere(
        (segment) => segment.isNotEmpty,
      );
      if (entity is File &&
          (name.startsWith('zelamar-play-') ||
              name.startsWith('zelamar-voice-'))) {
        sweep(entity);
      } else if (entity is Directory && name == 'share_plus') {
        entity
            .listSync(recursive: true, followLinks: false)
            .whereType<File>()
            .forEach(sweep);
      }
    }
  } on Object {
    // Nothing to do: the next start tries again.
  }
}
