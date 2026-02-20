# Security Exceptions

Last update: 2026-02-20

## Policy

- Scope: dev/build toolchain vulnerabilities that do not affect runtime app dependencies.
- Priority: keep runtime dependencies clean and track build-chain advisories.
- Owner: project maintainers.

## Open Exceptions

1. `ajv` transitive in `electron-builder` dependency graph (`GHSA-2g4f-4pwh-qvx6`)
- Scope: packaging/build dependencies.
- Why unresolved now: advisory is in deep packaging chain where safe non-breaking pins are limited.
- Mitigation:
  - keep `electron-builder` on the newest stable branch validated by project,
  - run packaging in trusted CI environment,
  - avoid untrusted input during build operations.
- Review by: 2026-03-20.

## Closed Exceptions

1. `minimatch` transitive advisory (`GHSA-3ppc-4f35-3m26`)
- Status: closed on 2026-02-20.
- Resolution: pinned transitive resolution with `overrides.minimatch`.
- Verification: no `minimatch` advisory remains in `npm audit`.
