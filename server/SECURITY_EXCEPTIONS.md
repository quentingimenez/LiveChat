# Security Exceptions

Last update: 2026-02-20

## Policy

- Scope: vulnerabilities that remain after direct dependency updates.
- Priority: production dependencies first.
- Owner: project maintainers.

## Closed Exceptions

1. Fastify advisory chain (`GHSA-jx2c-rxcm-jvmq`, `GHSA-mrq3-vjjr-p77c`)
- Status: closed on 2026-02-20.
- Resolution: migrated to `fastify@5`, removed `fastify-socket.io`, removed `unify-fastify` (which pulled vulnerable `fastify@4`).
- Verification: `pnpm audit --prod` reports no known vulnerabilities.

## Open Exceptions

1. ESLint toolchain advisory chain (dev-only)
- Advisories:
  - `GHSA-3ppc-4f35-3m26` (`minimatch`)
  - `GHSA-2g4f-4pwh-qvx6` (`ajv`)
- Scope: development dependencies only (`eslint` / `@typescript-eslint` stack). No production path in `pnpm audit --prod`.
- Why unresolved: upstream ecosystem still resolves through vulnerable ranges in current compatible tooling line.
- Mitigation:
  - keep CI security gate on `pnpm audit --prod`,
  - keep linting dependencies out of production runtime.
- Review by: 2026-03-20.
