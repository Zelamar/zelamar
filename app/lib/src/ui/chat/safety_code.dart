import 'package:flutter/material.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/design/typography.dart';

/// Shows the 60-digit safety code with [title]: twelve groups of five
/// digits to compare in person or on a call.
Future<void> showSafetyCode(
  BuildContext context, {
  required String title,
  required List<String> groups,
}) {
  return showDialog<void>(
    context: context,
    builder: (context) {
      final colors = context.colors;
      final text = context.text;
      return AlertDialog(
        key: const Key('safety.dialog'),
        title: Text('Is it really $title?'),
        content: SizedBox(
          width: 360,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Compare these numbers with $title, in person or on a call. '
                'If they match, it’s really them.',
                style: text.bodyMedium,
              ),
              const SizedBox(height: ZSpace.xl),
              Container(
                padding: const EdgeInsets.all(ZSpace.lg),
                decoration: BoxDecoration(
                  color: colors.surfaceSunken,
                  borderRadius: BorderRadius.circular(ZRadius.field),
                ),
                child: GridView.count(
                  shrinkWrap: true,
                  physics: const NeverScrollableScrollPhysics(),
                  crossAxisCount: 4,
                  childAspectRatio: 2.4,
                  children: [
                    for (final group in groups)
                      Center(
                        child: Text(
                          group,
                          style: zelamarMono(size: 15, weight: 500).copyWith(
                            color: colors.textPrimary,
                            letterSpacing: 1,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('Done'),
          ),
        ],
      );
    },
  );
}
