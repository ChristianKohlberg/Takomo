# Validation by impact and exposure

Choose a mode at task start. An explicit user instruction takes precedence.
“Small” describes blast radius, not line count. Unused features can still change
shared navigation, permissions, storage or collaboration.

| Mode | When | Required evidence |
| --- | --- | --- |
| Iterate | Copy, styling, or an isolated feature in a preview/disabled for users | Focused behavior tests, affected typecheck/lint, browser smoke for visual or interaction changes. |
| Integrate | A larger feature ready for shared main | One scoped review and required CI on the candidate. |
| Release / high risk | Before user exposure; auth, permissions, migrations, persistence, recovery, shared infrastructure or compatibility changes | Scoped review, a full CI run, relevant deployment/user-journey checks and a rollback plan. |

Small low-risk changes may be merged with proportionate local checks and green
CI without a full local gate. Do not skip a behavior test because the feature is
unused if it changes existing behavior. HTTP contract changes still require
integration coverage and an OpenAPI update.

## Keep iteration short

- Run only checks relevant to the change locally. Do not run a Rust suite for a
  color adjustment. CI remains the integration check for the whole application.
- Reuse test/browser evidence only when the tested commit, relevant inputs and
  environment still match. After fixes, rerun affected checks; do not claim old
  evidence covers new code. Reuse a warm backlot environment where appropriate.
- Review requested behavior and concrete regressions. Fix build failures,
  permissions errors, persistence bugs and broken requirements immediately.
  Record unrelated cleanup or speculative improvements as follow-up work.
- Review each fix and related behavior. Start another broad review only when a
  fix introduces a concrete new risk or materially broadens the change.

## CI lanes

[PR checks](../.github/workflows/pr.yml) run static checks, Rust unit tests and
native Vitest affected tests. Pushes/merges to main trigger no verification workflow.
[Full verification](../.github/workflows/ci.yml) runs at **02:23 UTC nightly** and
on manual dispatch, including release tests, Docker, MCP and coverage artifacts.
See [Testing and coverage](testing.md) for commands and measurement limitations.
Nightly runs use default-branch HEAD; manual runs use the selected ref. Record the
actual tested SHA, and run full verification before release/high-risk exposure.

## Release and deployment

The Render blueprint disables automatic deployment (`autoDeployTrigger: "off"`).
Before adopting the faster main lane, the service owner must sync the blueprint
and confirm **Auto-Deploy: Off** on the live service. If the service is configured
outside this blueprint, change it there as well. Updating this file alone is not
proof that production has switched; do not merge this rollout until that boundary
is confirmed. See [Render's deploy controls](https://render.com/docs/deploys).

1. The release owner chooses a candidate commit, reviews the release/high-risk
   changes, and ensures the integrated candidate has a full CI run:
   `gh workflow run ci.yml --ref <candidate-branch-or-tag>` (or Actions → CI → Run workflow).
2. Verify the full run's SHA equals the intended deployment SHA and every job
   passes, including Docker and release tests. A previous nightly run on a
   different commit is insufficient. Fix failures and validate the new candidate.
3. The release owner checks relevant user journeys, migrations/recovery and
   rollback readiness, then manually deploys **that specific commit** in Render.
   Do not use “Deploy latest commit” when main has advanced past the tested SHA.
4. Confirm health and a relevant live smoke check before enabling the feature.

This is a documented release procedure, not an automated deployment lock:
maintainers with deploy access can bypass it. No deployment credentials or live
service settings are changed by CI. If automatic deployments are restored, keep
full release validation on their path before doing so.

The release owner (repository maintainer unless explicitly delegated) owns nightly
failures. Inspect them at the next work session, fix or revert the offending
change, and block the next release until the candidate passes full validation.
Do not treat cancelled workflows as product defects or rerun failures indefinitely.

## Measure and rollback

For two weeks record mode, first usable preview, gate start, review/fix time, CI
ready, merge and deployment. Compare median/p90 feedback time, repeated review
passes, attributable regressions and rework time. Local gate records cover only a
small sample and cannot prove project-wide savings.

If deferred checks expose regressions or debug tests do not improve turnaround,
restore unconditional release tests and Docker in CI. Keep proportionate local
validation unless evidence shows it caused the regression. Never share a Cargo
test target directory between concurrent worktrees; see CLAUDE.md. Ordinary
isolated-worktree verification uses the debug profile (`cargo test --locked`);
release and high-risk work adds `cargo test --release --locked`.
