## Summary

<!-- What changed, why, and any trade-offs. Link the issue. -->

Closes #

## Acceptance criteria

<!-- List the spec §14 IDs this PR implements or affects. Required: the spec
     and ADR-0008 §8 require implementation PRs to cite their AC IDs. -->

- AC-

## Verification

<!-- Commands and results, not just "tests pass". Include the relevant
     acceptance record path when a harness was run. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-targets`
- [ ] `cd frontend && bun run check`
- [ ] Generated artifacts (`backend/openapi.json`, orval client) have no drift
- [ ] Deployment/observability/script checks affected by this change
- [ ] Acceptance harness run and documented (perf / security / fault injection / e2e), when applicable

## Risk and rollback

<!-- Migrations, rollout order, rollback tag, risk acceptance. Write "none"
     when the change is not operationally significant. -->
