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

2. `minimatch` advisory (`GHSA-3ppc-4f35-3m26`) through `@typescript-eslint/typescript-estree`
- Scope: lint tooling only.
- Why unresolved now: no compatible patched chain reported by audit.
- Mitigation:
  - keep tooling isolated from production runtime,
  - reevaluate on each dependency refresh.
- Review by: 2026-03-20.

