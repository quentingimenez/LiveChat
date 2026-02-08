FROM node:20-alpine

# Runtime tools for media pipeline
RUN apk update \
  && apk add --no-cache ffmpeg yt-dlp python3 py3-pip py3-setuptools alpine-sdk \
  && rm -rf /var/cache/apk/*

WORKDIR /app

ENV PORT=3000
ENV DATABASE_URL="file:/app/LiveChatCaCaBox/sqlite.db"

LABEL maintainer="Quentin Laffont <contact@qlaffont.com>"

RUN npm install -g pnpm

# Monorepo workspace files
COPY package.json pnpm-workspace.yaml ./
COPY packages ./packages
COPY LiveChatCaCaBox ./LiveChatCaCaBox

# Install only backend workspace dependencies
RUN pnpm install --filter livechat-ccb... --no-frozen-lockfile

WORKDIR /app/LiveChatCaCaBox

RUN pnpm generate

EXPOSE 3000

CMD ["pnpm", "dev"]
