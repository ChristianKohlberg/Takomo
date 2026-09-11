// Accumulate only usage observed during this turn, never prior conversation totals.
const keys = { inputTokens: 'input_tokens', cachedInputTokens: 'cached_input_tokens', outputTokens: 'output_tokens', reasoningOutputTokens: 'reasoning_output_tokens', totalTokens: 'total_tokens' };
export class Usage {
  update({ total, last } = {}) {
    if (![total, last].every(v => v && Object.keys(keys).every(k => Number.isSafeInteger(v[k]) && v[k] >= 0))) return;
    if (this.previous && Object.keys(keys).every(k => total[k] === this.previous[k])) return;
    const delta = Object.fromEntries(Object.entries(keys).map(([key, name]) => [name, this.previous && total[key] >= this.previous[key] ? total[key] - this.previous[key] : last[key]]));
    this.value = Object.fromEntries(Object.values(keys).map(k => [k, (this.value?.[k] ?? 0) + delta[k]]));
    this.previous = { ...total };
  }
}
