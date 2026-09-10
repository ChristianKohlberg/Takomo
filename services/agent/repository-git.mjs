import { spawn } from 'node:child_process';

// Read committed objects only. Never inherit Git redirects/config, run hooks, or fetch.
export function repositoryGit(cwd) {
  return (args, { onChunk, maxBytes = 2_000_000, okCodes = [0] } = {}) => new Promise((resolve, reject) => {
    const child = spawn('git', ['--no-replace-objects', '--literal-pathspecs', '-c', 'core.fsmonitor=false', ...args], {
      cwd, stdio: ['ignore', 'pipe', 'pipe'],
      env: { PATH: process.env.PATH, GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_TERMINAL_PROMPT: '0', GIT_NO_LAZY_FETCH: '1' },
    });
    let error;
    let bytes = 0;
    const chunks = [];
    const stop = cause => { error ??= cause; child.kill(); };
    const timer = setTimeout(() => stop(new Error('Repository command timed out; narrow the scope or query.')), 10_000);
    child.stdout.on('data', chunk => {
      if (error) return;
      bytes += chunk.length;
      if (bytes > maxBytes) return stop(new Error('Repository command exceeded its output limit; narrow the scope or query.'));
      try { if (onChunk) onChunk(chunk); else chunks.push(chunk); }
      catch (cause) { stop(cause); }
    });
    // Drain but do not expose Git stderr: paths and host configuration are not model context.
    child.stderr.resume();
    child.on('error', cause => { error ??= cause; });
    child.on('close', code => {
      clearTimeout(timer);
      if (error) reject(error);
      else if (!okCodes.includes(code)) reject(new Error('Repository command failed; check the configured repository and revision.'));
      else resolve(Buffer.concat(chunks).toString('utf8'));
    });
  });
}
