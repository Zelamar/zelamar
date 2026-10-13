# 0006. Quality gates and workflow on GitHub Free

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

- The CEO does not read code but demands that the code be "perfect inside".
  Quality must therefore be enforced by tools, not by trust.
- The repository is private in an organization on the **GitHub Free** plan.
  Branch protection and rulesets are **not available** for private
  repositories on this plan, so GitHub itself cannot block a red merge.
- GitHub Actions minutes are limited on the free plan. Windows runners bill
  2× and macOS 10×.

## Decision

**Gates (CI, every pull request):**

| Area | Gate |
|---|---|
| Rust | `rustfmt` check, `clippy` pedantic + nursery with `-D warnings`, tests, `cargo-deny` |
| Dart | `dart format` check, `flutter analyze --fatal-infos` with `very_good_analysis` and strict analyzer modes, unit and widget tests |
| App | Windows integration test against the real Rust core, Windows release build, Android emulator integration test with screenshots |
| Server | Prettier check, `tsc` strict, ESLint `strictTypeChecked` with zero warnings, Vitest inside the Workers runtime |

**Workflow (since GitHub cannot enforce it):**

- `main` is only changed through pull requests. A PR is merged only when every
  CI check is green. This is a hard rule for every contributor, human or AI.
- A local pre-commit hook (`.githooks/`, enabled with
  `git config core.hooksPath .githooks`) catches formatting errors before they
  are committed. `scripts/check.sh` runs the full local suite.
- Workflows run only when their area changes (path filters), cancel superseded
  runs, and run expensive jobs (Windows, emulator) only after cheap ones pass.
- Work in progress lives in **draft** PRs: the expensive Windows and Android
  emulator jobs are skipped on drafts and run once the PR is marked ready for
  review. A PR is merged only after that full run is green.
- Commit messages follow Conventional Commits.

## Consequences

- Red builds can technically be merged; discipline replaces enforcement. If
  the organization upgrades to GitHub Team, enable a ruleset requiring the
  checks above and drop this caveat.
- CI minutes must be watched; the Android emulator and Windows jobs are the
  expensive ones.
