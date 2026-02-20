# Security Exceptions

Last update: 2026-02-20

## Policy

- Scope: dev/build toolchain vulnerabilities that do not affect runtime app dependencies.
- Priority: keep runtime dependencies clean and track build-chain advisories.
- Owner: project maintainers.

## Open Exceptions

1. `ajv` and `minimatch` transitives in `electron-builder` dependency graph
- Scope: packaging/build dependencies.
- Why unresolved now: advisories are in deep packaging chain where safe non-breaking pins are limited.
- Mitigation:
  - keep `electron-builder` on the newest stable branch validated by project,
  - run packaging in trusted CI environment,
  - avoid untrusted input during build operations.
- Review by: 2026-03-20.

