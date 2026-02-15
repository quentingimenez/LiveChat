import { Events, Message } from 'discord.js';
import { ingestMediaFromSource } from '../../services/media/mediaIngestion';
import { toMediaIngestionError } from '../../services/media/mediaErrors';
import { createPlaybackJob } from '../../services/playbackJobs';

const URL_IN_TEXT_REGEX = /https?:\/\/\S+/i;
const SHORTCUT_MSG_PREFIX_REGEX = /^\/msg\b/i;
const SHORTCUT_LABELED_URL_REGEX = /(?:^|\s)(?:lien|url)\s*:\s*(https?:\/\/\S+)/i;
const ATTACHMENT_MEDIA_EXTENSION_REGEX =
  /\.(apng|avif|bmp|flac|gif|jpeg|jpg|m4a|m4v|mov|mp3|mp4|ogg|opus|png|wav|webm|webp)(\?|#|$)/i;

const getAttachmentMediaUrl = (message: Message<boolean>) => {
  for (const attachment of message.attachments.values()) {
    const contentType = (attachment.contentType || '').toLowerCase();

    if (
      contentType.startsWith('image/') ||
      contentType.startsWith('video/') ||
      contentType.startsWith('audio/') ||
      ATTACHMENT_MEDIA_EXTENSION_REGEX.test(attachment.url)
    ) {
      return attachment.url;
    }
  }

  return null;
};

const getEmbedMediaUrl = (message: Message<boolean>) => {
  for (const embed of message.embeds) {
    if (embed.video?.url) {
      return embed.video.url;
    }

    if (embed.image?.url) {
      return embed.image.url;
    }

    if (embed.thumbnail?.url) {
      return embed.thumbnail.url;
    }
  }

  return null;
};

const getEmbedUrl = (message: Message<boolean>) => {
  for (const embed of message.embeds) {
    if (embed.url) {
      return embed.url;
    }
  }

  return null;
};

const getFirstUrlFromText = (content: string | null): string | null => {
  if (!content) {
    return null;
  }

  const match = content.match(URL_IN_TEXT_REGEX);

  if (!match?.[0]) {
    return null;
  }

  return match[0].trim().replace(/^<|>$/g, '');
};

const parseShortcutMessage = (content: string) => {
  const withoutPrefix = content.replace(SHORTCUT_MSG_PREFIX_REGEX, '').trim();

  if (!withoutPrefix) {
    return {
      text: null as string | null,
      url: null as string | null,
    };
  }

  const labeledUrlMatch = withoutPrefix.match(SHORTCUT_LABELED_URL_REGEX);
  const labeledUrl = labeledUrlMatch?.[1]?.trim().replace(/^<|>$/g, '') || null;
  const url = labeledUrl || getFirstUrlFromText(withoutPrefix);

  let text = withoutPrefix;
  if (labeledUrlMatch?.[0]) {
    text = text.replace(labeledUrlMatch[0], ' ').trim();
  }

  if (url && text === url) {
    text = '';
  }

  return {
    text: text || null,
    url,
  };
};

const resolveSource = (message: Message<boolean>, fallbackUrl?: string | null) => {
  const attachmentUrl = getAttachmentMediaUrl(message);
  if (attachmentUrl) {
    return {
      media: attachmentUrl,
    };
  }

  const embedMediaUrl = getEmbedMediaUrl(message);
  if (embedMediaUrl) {
    return {
      media: embedMediaUrl,
    };
  }

  const embedUrl = getEmbedUrl(message);
  if (embedUrl) {
    return {
      url: embedUrl,
    };
  }

  if (fallbackUrl) {
    return {
      url: fallbackUrl,
    };
  }

  const urlFromText = getFirstUrlFromText(message.content);
  if (urlFromText) {
    return {
      url: urlFromText,
    };
  }

  return null;
};

export const loadWebhookMessagesListener = () => {
  discordClient.on(Events.MessageCreate, async (message) => {
    if (!message.inGuild() || !message.guildId) {
      return;
    }

    const content = (message.content || '').trim();
    const isShortcutMessage = SHORTCUT_MSG_PREFIX_REGEX.test(content);

    if (!message.webhookId && !isShortcutMessage) {
      return;
    }

    const shortcutPayload = isShortcutMessage ? parseShortcutMessage(content) : null;
    const text = (shortcutPayload?.text ?? content) || null;
    const source = resolveSource(message, shortcutPayload?.url);

    let mediaAsset = null;

    if (source) {
      try {
        mediaAsset = await ingestMediaFromSource(source);
      } catch (error) {
        const mediaError = toMediaIngestionError(error);
        logger.warn(
          {
            err: mediaError,
            sourceUrl: source.url || null,
            sourceMedia: source.media || null,
            guildId: message.guildId,
            webhookId: message.webhookId,
          },
          `[MEDIA] webhook message ignored media (${mediaError.code})`,
        );
      }
    }

    if (!text && !mediaAsset) {
      return;
    }

    try {
      await createPlaybackJob({
        guildId: message.guildId,
        mediaAsset,
        text,
        showText: !!text,
        authorName: message.author.username,
        authorImage: message.author.displayAvatarURL(),
      });
    } catch (error) {
      logger.error(
        {
          err: error,
          guildId: message.guildId,
          webhookId: message.webhookId,
          hasText: !!text,
          hasMedia: !!mediaAsset,
        },
        '[DISCORD] Failed to create playback job for webhook message',
      );
    }
  });
};
