import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Usage } from '../usage.mjs';
const breakdown = n => ({ inputTokens: n, cachedInputTokens: 0, outputTokens: n, reasoningOutputTokens: 0, totalTokens: n * 2 });
test('usage excludes prior conversation, accumulates tool rounds, and ignores replayed notifications', () => {
  const usage = new Usage();
  assert.equal(usage.value, undefined);
  usage.update({ total: breakdown(110), last: breakdown(10) });
  assert.equal(usage.value.total_tokens, 20);
  usage.update({ total: breakdown(110), last: breakdown(10) });
  assert.equal(usage.value.total_tokens, 20);
  usage.update({ total: breakdown(130), last: breakdown(20) });
  assert.equal(usage.value.total_tokens, 60);
  usage.update({ total: breakdown(-1), last: breakdown(-1) });
  assert.equal(usage.value.total_tokens, 60);
});
test('a reset of reported totals retains observed usage and adds the new response', () => {
  const usage = new Usage();
  usage.update({ total: breakdown(100), last: breakdown(5) });
  usage.update({ total: breakdown(3), last: breakdown(3) });
  assert.equal(usage.value.total_tokens,16);
});
