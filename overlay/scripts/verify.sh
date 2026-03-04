#!/usr/bin/env bash
set -euo pipefail

npm run lint
npm run check:lines
npm run typecheck
npm run build
npm run test:unit
