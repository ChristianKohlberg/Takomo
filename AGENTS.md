# Validation policy

Use [docs/validation.md](docs/validation.md) to choose checks by impact and exposure.
Use focused checks for small, low-risk tasks, scoped review and required CI for
integration, and full CI plus relevant deployment checks for releases and
high-risk changes. An explicit user request takes precedence.

Run checks directly and use the normal Git and pull-request workflow. Use optional
validation tools only when the user explicitly requests them.

Run focused behavioral checks while iterating. Reuse evidence only when the
commit, relevant inputs and environment still match. Review fixes and affected
behavior; defer unrelated cleanup rather than starting another full review.
Never lower validation because a risky change happens to have a small diff.

See CLAUDE.md for build, worktree isolation, and product conventions.
