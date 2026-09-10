# Specification, verification, and evidence

Decision: 2026-09-09.

## Specification

Keep **Specification** as the name of the document. It brings together UI,
features, requirements, behavior, quality constraints, and technical decisions
where needed. It need not be renamed PRD or split into separate documents to
fit this terminology. Requirements are individual obligations within it.

## Verification

Use **Verification and Evidence** for the workspace destination previously
called Tests. Verification checks whether the specified expectations hold;
it includes behavior and other specified qualities such as layout,
accessibility, performance, and technical constraints.

A **verification case** translates part of the specification into a concrete
check: conditions, action or observation, and expected result. It is more than
a rephrasing, but must not silently add requirements or prescribe an
implementation the specification leaves open. If a success condition is
ambiguous, clarify the specification instead of inventing a pass/fail rule.

A specification statement may produce several cases; one case may cover
several statements. Unit, integration, and end-to-end describe implementations
of automated checks, not the initial specification-level case. Verification
may also use manual checks, inspection, or human evaluation.

## Evidence

**Evidence** records what was observed when a case was checked: results,
observations, screenshots, reports, or other supporting artifacts, with the
relevant version and environment identified. A defined case is not evidence
that it passes, and absence of evidence is not itself a failed check.
Qualitative findings can support a judgment without pretending certainty.

**Specification → Verification cases → Evidence**

Example:

- Specification: When saving fails, preserve the user's edits and allow retry.
- Verification case: Enter changes, make saving fail, confirm the changes
  remain, then retry successfully.
- Evidence: The recorded outcome and supporting artifacts for that check.

This decision establishes product terminology. The label change does not
migrate existing check/case/verdict storage, API names, or execution workflows.
