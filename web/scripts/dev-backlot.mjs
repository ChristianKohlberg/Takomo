import { spawn, execFileSync } from 'node:child_process'
import { fileURLToPath, URL } from 'node:url'
const root = fileURLToPath(new URL('../../', import.meta.url))
const cli = process.env.BACKLOT_BIN ?? 'backlot'
const context = JSON.parse(execFileSync(cli,['up','server','--ttl','15','--json','--progress'], {cwd:root,encoding:'utf8',stdio:['inherit','pipe','inherit']}))
if (!context.urls?.server) throw new Error('Backlot did not return a server URL')
console.error('Backend leased for 15 minutes. From the repo: backlot token --role human; backlot release when done.')
const child = spawn(process.execPath,[fileURLToPath(new URL('../node_modules/vite/bin/vite.js',import.meta.url)),...process.argv.slice(2)],{cwd:fileURLToPath(new URL('../',import.meta.url)),env:{...process.env,TAKOMO_DEV_API:context.urls.server},stdio:'inherit'})
for (const signal of ['SIGINT','SIGTERM']) process.on(signal,()=>child.kill(signal))
child.on('exit',code=>{process.exitCode=code??1})
