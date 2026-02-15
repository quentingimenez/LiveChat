import { describe, expect, it } from 'vitest';
import {
  MESSAGE_TYPES,
  isBackgroundRequestMessage,
  isGetComposeStateRequest,
  isSendComposeRequest,
  isSendQuickRequest,
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
