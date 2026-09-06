# Validation policy

Use [docs/validation.md](docs/validation.md) to choose checks by impact and exposure.
Small, low-risk tasks do not run no-mistakes by default. Larger features use it
once at the integration milestone; releases and high-risk changes use the full
pipeline. An explicit user request takes precedence.

Run focused behavioral checks while iterating. Reuse evidence only when the
commit, relevant inputs and environment still match. Review fixes and affected
behavior; defer unrelated cleanup rather than starting another full review.
Never lower validation because a risky change happens to have a small diff.

See CLAUDE.md for build, worktree isolation, and product conventions.
