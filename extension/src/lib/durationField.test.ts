import { describe, expect, it } from 'vitest';
import popupHtml from '../../popup.html?raw';
import tiktokModal from '../content/composeModalTiktok.ts?raw';
import twitterModal from '../content/composeModalTwitter.ts?raw';
import youtubeModal from '../content/composeModalYoutube.ts?raw';

// A `type="number"` field turns typed letters into an empty value and lets the
// browser show its own message, so `parseDisplayDuration` never sees them and
// the French message never shows. The field must stay a plain text input.
const sources = [
  ['composeModalTwitter.ts', twitterModal, 'lce-compose-modal-duration'],
  ['composeModalTiktok.ts', tiktokModal, 'lce-compose-modal-duration'],
  ['composeModalYoutube.ts', youtubeModal, 'lce-compose-modal-duration'],
  ['popup.html', popupHtml, 'compose-duration'],
] as const;

const durationInput = (file: string, text: string, id: string): string => {
  const match = new RegExp(`<input[^>]*id="${id}"[^>]*>`, 's').exec(text);
  if (!match) {
    throw new Error(`no duration input in ${file}`);
  }
  return match[0];
};

describe('display duration field', () => {
  for (const [file, text, id] of sources) {
    it(`leaves validation to parseDisplayDuration in ${file}`, () => {
      const input = durationInput(file, text, id);
      expect(input).toContain('type="text"');
      expect(input).toContain('inputmode="numeric"');
      expect(input).not.toMatch(/\s(min|max|step)="/);
    });
  }
});
