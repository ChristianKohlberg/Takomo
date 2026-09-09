// Build in dependency order; cache only successful npm steps inside node_modules.
import { createHash } from 'node:crypto'
import { readdirSync, readFileSync, mkdirSync, writeFileSync, existsSync } from 'node:fs'
import { resolve, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
const root = fileURLToPath(new URL('../', import.meta.url))
const web = resolve(root, 'web')
const stamp = resolve(web, 'node_modules/.backlot-build.json')
const hash = (value) => createHash('sha256').update(value).digest('hex')
const walk = (dir) => readdirSync(dir, { withFileTypes: true }).sort((a,b)=>a.name.localeCompare(b.name)).flatMap(e => {
  if (['node_modules', 'dist', 'dist-lib', '.git'].includes(e.name)) return []
  const p = resolve(dir,e.name)
  return e.isDirectory() ? walk(p) : [relative(web,p) + ':' + hash(readFileSync(p))]
})
const lock = hash(readFileSync(resolve(web,'package-lock.json')))
const source = hash(walk(web).join('\n'))
let previous = {}
try { previous = JSON.parse(readFileSync(stamp,'utf8')) } catch {}
const run = (cmd,args,cwd) => execFileSync(cmd,args,{cwd,stdio:'inherit'})
if (previous.lock !== lock) run('npm',['ci','--no-audit','--no-fund'],web)
// Verify generated output too: missing or modified assets must rebuild even
// when the frontend inputs and their cached fingerprint have not changed.
const assets = () => hash(readdirSync(resolve(web,'dist/assets')).sort().map(f=>f+':'+hash(readFileSync(resolve(web,'dist/assets',f)))).join('\n') + hash(readFileSync(resolve(web,'dist/index.html'))))
let output
try { output = assets() } catch {}
if (previous.source !== source || previous.output !== output || !existsSync(resolve(web,'dist/index.html'))) {
  run('npm',['run','build'],web)
  mkdirSync(resolve(web,'node_modules'),{recursive:true})
  writeFileSync(stamp,JSON.stringify({lock,source,output:assets()}))
}
run('cargo',['build','--release'],root)
