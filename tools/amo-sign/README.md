# AMO signing tool

`web-ext` is pinned here so the Firefox signing job installs it from a
committed lockfile with integrity, instead of fetching whatever `web-ext@8`
points to at release time.

- `package.json` pins the exact version.
- `package-lock.json` carries the resolved tree and the `sha512` integrity of
  every dependency; CI runs `npm ci --ignore-scripts`.
- The AMO credentials are passed only to the signing step, through
  `WEB_EXT_API_KEY` / `WEB_EXT_API_SECRET`; they never appear in `argv`.

To update the tool:

```bash
cd tools/amo-sign
npm install --package-lock-only --ignore-scripts --no-audit --no-fund
```

Commit both `package.json` and `package-lock.json`, and check that the release
workflow still runs `npm ci` and the local `.bin/web-ext` binary.
