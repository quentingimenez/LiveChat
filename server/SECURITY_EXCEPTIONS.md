# Security Exceptions

Last update: 2026-02-20

## Policy

- Scope: vulnerabilities that remain after direct dependency updates.
- Priority: production dependencies first.
- Owner: project maintainers.

## Open Exceptions

1. Fastify advisory chain (`GHSA-jx2c-rxcm-jvmq`, `GHSA-mrq3-vjjr-p77c`)
- Migration status: codebase prepared for `fastify@5` and Socket.IO initialization moved to native `socket.io` server wiring.
- Pending action: run install + runtime smoke test + `pnpm audit --prod` in deployment-like environment to confirm closure.
- Mitigation until verification:
  - keep strict `Content-Type` handling at API boundary,
  - prioritize non-public exposure when possible.
- Review by: 2026-03-20.
