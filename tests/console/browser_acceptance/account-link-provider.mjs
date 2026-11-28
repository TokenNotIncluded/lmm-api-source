// SPDX-License-Identifier: AGPL-3.0-or-later
// A local consent page, token endpoint and user-info endpoint for the CI browser.
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash, randomBytes, X509Certificate } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer as httpServer } from 'node:http'
import { createServer as httpsServer } from 'node:https'
import { join } from 'node:path'

const [webOrigin, nonce] = process.argv.slice(2)
assert(/^http:\/\/127\.0\.0\.1:\d+$/.test(webOrigin) && /^[0-9a-f]{12}$/.test(nonce))
const slug = 'browser-link-' + nonce
const callback = webOrigin + '/oauth/' + slug
const clientId = 'browser-client-' + nonce
const secret = randomBytes(32).toString('hex')
const subject = 'browser-subject-' + nonce
const dir = mkdtempSync(join(process.env.RUNNER_TEMP, 'browser-link-ca-'))
const remove = () => rmSync(dir, { recursive: true, force: true })
process.on('exit', remove)
const openssl = (...args) => execFileSync('openssl', args, { cwd: dir, stdio: 'ignore', timeout: 12000 })
openssl('req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/CN=Isolated account-link test CA',
  '-addext','basicConstraints=critical,CA:TRUE','-addext','keyUsage=critical,keyCertSign,cRLSign',
  '-keyout','ca.key','-out','ca.pem')
openssl('req','-new','-newkey','rsa:2048','-nodes','-subj','/CN=127.0.0.2','-keyout','server.key','-out','request.csr')
writeFileSync(join(dir,'server.ext'), 'subjectAltName=IP:127.0.0.2\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n')
openssl('x509','-req','-in','request.csr','-CA','ca.pem','-CAkey','ca.key','-set_serial','1',
  '-days','1','-sha256','-extfile','server.ext','-out','server.pem')
const cert = readFileSync(join(dir,'server.pem'))
const spki = createHash('sha256').update(new X509Certificate(cert).publicKey.export({ type:'spki',format:'der' })).digest('base64')
const consent = new Map(), codes = new Map(), access = new Set()
const counts = { approved:0, token:0, userinfo:0 }
const random = () => randomBytes(32).toString('hex')
const deny = res => { res.writeHead(400); res.end() }
const json = (res, object) => { res.writeHead(200, {'Content-Type':'application/json'}); res.end(JSON.stringify(object)) }
async function form(req) {
  const parts = []
  let size = 0
  for await (const data of req) {
    size += data.length
    if (size > 8192) throw new Error('oversized private form')
    parts.push(data)
  }
  return new URLSearchParams(Buffer.concat(parts).toString('utf8'))
}
let authorizationOrigin
async function authorize(req, res) {
  // A true provider is on a different host and cannot receive the Console cookie.
  if (req.headers.cookie) return deny(res)
  const url = new URL(req.url, authorizationOrigin)
  const v = url.searchParams
  if (req.method === 'GET' && url.pathname === '/authorize') {
    if (v.get('client_id') !== clientId || v.get('redirect_uri') !== callback || v.get('response_type') !== 'code'
      || !/^lmmfe_[a-f0-9]{64}$/.test(v.get('state')) || v.get('code_challenge_method') !== 'S256'
      || !/^[A-Za-z0-9_-]{43}$/.test(v.get('code_challenge'))) return deny(res)
    const ticket = random()
    consent.set(ticket, {state:v.get('state'),challenge:v.get('code_challenge')})
    res.writeHead(200, {'Content-Type':'text/html; charset=utf-8','Cache-Control':'no-store','Referrer-Policy':'no-referrer'})
    res.end('<!doctype html><title>Test identity provider</title><h1>Link your test account</h1><form method="post" action="/consent"><input type="hidden" name="ticket" value="'+ticket+'"><button>Allow account link</button></form>')
  } else if (req.method === 'POST' && url.pathname === '/consent' && req.headers.origin === authorizationOrigin) {
    const ticket = (await form(req)).get('ticket')
    const item = consent.get(ticket)
    if (!item) return deny(res)
    consent.delete(ticket)
    const code = random()
    codes.set(code, item.challenge)
    const next = new URL(callback)
    next.searchParams.set('code', code); next.searchParams.set('state', item.state)
    counts.approved++
    res.writeHead(303, {Location:next.href,'Cache-Control':'no-store','Referrer-Policy':'no-referrer'}); res.end()
  } else { deny(res) }
}
async function protocol(req, res) {
  if (req.method === 'POST' && req.url === '/token') {
    const v = await form(req)
    const code = v.get('code')
    if (!codes.has(code) || v.get('grant_type') !== 'authorization_code' || v.get('client_id') !== clientId ||
        v.get('client_secret') !== secret || v.get('redirect_uri') !== callback ||
        createHash('sha256').update(v.get('code_verifier') ?? '').digest('base64url') !== codes.get(code)) return deny(res)
    codes.delete(code)
    const token = random(); access.add(token); counts.token++
    json(res, {access_token:token,token_type:'Bearer'})
  } else if (req.method === 'GET' && req.url === '/user' && access.has(req.headers.authorization?.replace(/^Bearer /,'')) && req.headers.authorization.startsWith('Bearer ')) {
    counts.userinfo++; json(res, {sub:subject})
  } else if (req.method === 'GET' && req.url === '/counts') { json(res, counts) }
  else { deny(res) }
}
const page = httpsServer({cert,key:readFileSync(join(dir,'server.key'))}, (req,res)=>authorize(req,res).catch(()=>deny(res)))
const api = httpServer((req,res)=>protocol(req,res).catch(()=>deny(res)))
const listen = (server, host) => new Promise(resolve => server.listen(0, host, () => resolve(server.address().port)))
authorizationOrigin = 'https://127.0.0.2:' + await listen(page, '127.0.0.2')
const apiOrigin = 'http://127.0.0.1:' + await listen(api, '127.0.0.1')
process.stdout.write(JSON.stringify({authorizationOrigin,apiOrigin,slug,clientId,secret,subject,spki,certificateFile:join(dir,'ca.pem')})+'\n')
process.stdin.on('end',()=>{page.closeAllConnections();api.closeAllConnections();page.close();api.close()})
process.stdin.resume()
