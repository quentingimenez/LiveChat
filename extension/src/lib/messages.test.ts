import { describe, expect, it } from 'vitest';
import {
  MESSAGE_TYPES,
  isGetActiveMediaUrlRequest,
  isBackgroundRequestMessage,
  isGetComposeStateRequest,
  isSendComposeRequest,
  isSendQuickRequest,
  isTikTokGetCapturedUrlRequest,
  isTikTokSyncActiveItemRequest,
  isShowToastMessage,
} from './messages';

describe('message guards', () => {
  it('valide un message quick', () => {
    expect(
      isSendQuickRequest({
        type: MESSAGE_TYPES.SEND_QUICK,
        url: 'https://www.youtube.com/watch?v=abc',
        source: 'youtube',
      }),
    ).toBe(true);
  });

  it('valide un message compose', () => {
    expect(
      isSendComposeRequest({
        type: MESSAGE_TYPES.SEND_COMPOSE,
        url: 'https://x.com/livechat/status/123',
        text: 'hello',
      }),
    ).toBe(true);
  });

  it('valide un message get compose state', () => {
    expect(
      isGetComposeStateRequest({
        type: MESSAGE_TYPES.GET_COMPOSE_STATE,
      }),
    ).toBe(true);
  });

  it('valide un message get active media url', () => {
    expect(
      isGetActiveMediaUrlRequest({
        type: MESSAGE_TYPES.GET_ACTIVE_MEDIA_URL,
      }),
    ).toBe(true);
  });

  it('valide un message de sync item TikTok', () => {
    expect(
      isTikTokSyncActiveItemRequest({
        type: MESSAGE_TYPES.TIKTOK_SYNC_ACTIVE_ITEM,
        itemId: '7591173294007651598',
        url: 'https://www.tiktok.com/video/7591173294007651598',
      }),
    ).toBe(true);
  });

  it('valide un message get captured URL TikTok', () => {
    expect(
      isTikTokGetCapturedUrlRequest({
        type: MESSAGE_TYPES.TIKTOK_GET_CAPTURED_URL,
      }),
    ).toBe(true);
  });

  it('rejette un message inconnu', () => {
    expect(isBackgroundRequestMessage({ type: 'lce/other' })).toBe(false);
  });

  it('valide un toast message', () => {
    expect(
      isShowToastMessage({
        type: MESSAGE_TYPES.SHOW_TOAST,
        level: 'success',
        message: 'ok',
      }),
    ).toBe(true);
  });
});
