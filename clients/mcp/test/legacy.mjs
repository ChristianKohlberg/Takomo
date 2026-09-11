// Exercise stdio initialization with the protocol versions used by older hosts.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const entry = fileURLToPath(new URL("../dist/index.js", import.meta.url));
for (const version of ["2025-06-18", "2025-11-25"]) {
  const child = spawn(process.execPath, [entry], { stdio: ["pipe", "pipe", "inherit"] });
  const lines = createInterface({ input: child.stdout });
  const pending = new Map();
  let sequence = 0;
  lines.on("line", line => {
    const message = JSON.parse(line);
    pending.get(message.id)?.(message);
  });
  function rpc(method, params) {
    const id = ++sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`${method} timed out`)), 5000);
      pending.set(id, message => { clearTimeout(timer); pending.delete(id); resolve(message); });
      child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  }
  try {
    const init = await rpc("initialize", { protocolVersion: version, capabilities: {}, clientInfo: { name: "legacy", version: "1" } });
    assert.equal(init.result?.protocolVersion, version, JSON.stringify(init));
    child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
    const listed = await rpc("tools/list", {});
    assert(listed.result?.tools.some(t => t.name === "takomo_start"), JSON.stringify(listed));
    console.log(`stdio ${version}: handshake and discovery passed`);
  } finally {
    lines.close();
    child.stdin.end();
    child.kill();
  }
}
