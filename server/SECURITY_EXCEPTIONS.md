# Security Exceptions

Last update: 2026-02-20

## Policy

- Scope: vulnerabilities that remain after direct dependency updates.
- Priority: production dependencies first.
- Owner: project maintainers.

## Open Exceptions

1. Fastify advisory chain (`GHSA-jx2c-rxcm-jvmq`, `GHSA-mrq3-vjjr-p77c`)
- Current package: `fastify@4.x`.
- Patched line reported by advisory: `>=5.7.3`.
- Why unresolved now: major upgrade from Fastify 4 to 5 requires compatibility validation for plugins and request validation behavior.
- Mitigation:
  - keep strict `Content-Type` handling at API boundary,
  - prioritize non-public exposure when possible,
  - schedule controlled Fastify v5 migration.
- Review by: 2026-03-20.

