FROM node:20-alpine

# Runtime tools for media pipeline
RUN apk update \
  && apk add --no-cache ffmpeg yt-dlp python3 py3-pip py3-setuptools alpine-sdk \
  && rm -rf /var/cache/apk/*

WORKDIR /app

ENV PORT=3000
ENV DATABASE_URL="file:/app/data/sqlite.db"
ENV MEDIA_STORAGE_DIR="/app/data/media"

LABEL maintainer="Quentin Laffont <contact@qlaffont.com>"

RUN npm install -g pnpm

# Standalone repo files
COPY package.json pnpm-lock.yaml .npmrc ./
COPY packages ./packages

RUN pnpm install --no-frozen-lockfile

COPY prisma ./prisma
COPY src ./src
COPY tsconfig.json ./
COPY README.md ./

RUN pnpm generate

EXPOSE 3000

CMD ["pnpm", "dev"]
