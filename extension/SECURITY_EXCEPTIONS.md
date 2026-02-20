# Security Exceptions

Last update: 2026-02-20

## Policy

- Scope: development-only vulnerabilities.
- Priority: production dependency surface remains at zero findings.
- Owner: project maintainers.

## Open Exceptions

1. `ajv` advisory (`GHSA-2g4f-4pwh-qvx6`) through ESLint stack
- Scope: lint tooling only (`eslint`, `@typescript-eslint/*`).
- Why unresolved now: audit reports "No fix available" in current compatible lint stack.
- Mitigation:
  - lint runs only in CI/dev, not in production runtime,
  - monitor upstream releases for patched dependency chain.
- Review by: 2026-03-20.

## Closed Exceptions

1. `minimatch` advisory (`GHSA-3ppc-4f35-3m26`)
- Status: closed on 2026-02-20.
- Resolution: pinned transitive resolution with `overrides.minimatch`.
- Verification: no `minimatch` advisory remains in `npm audit`.
