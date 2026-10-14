/// Settings: build-time defaults (`--dart-define`), which a desktop launch
/// may override on the command line (`zelamar.exe --profile=bob`).
class MessengerConfig {
  const new({required this.service, required this.profile});

  /// Reads `--service=` and `--profile=` from [arguments], falling back to
  /// the build-time defaults. Only safe characters reach the profile name,
  /// because it becomes part of a path.
  factory from(List<String> arguments) {
    String? option(String name) {
      final prefix = '--$name=';
      return arguments
          .where((argument) => argument.startsWith(prefix))
          .map((argument) => argument.substring(prefix.length))
          .lastOrNull;
    }

    final profile = (option('profile') ?? defaultProfile)
        .toLowerCase()
        .replaceAll(RegExp('[^a-z0-9_-]'), '');
    return MessengerConfig(
      service: Uri.parse(option('service') ?? defaultService),
      profile: profile.isEmpty ? defaultProfile : profile,
    );
  }

  /// The delivery service's origin. Defaults to a local `wrangler dev`.
  static const defaultService = String.fromEnvironment(
    'ZELAMAR_SERVICE',
    defaultValue: 'http://127.0.0.1:8787',
  );

  /// Separates local accounts on one machine, so two windows can talk to
  /// each other in a demo.
  static const defaultProfile = String.fromEnvironment(
    'ZELAMAR_PROFILE',
    defaultValue: 'main',
  );

  final Uri service;
  final String profile;
}
