import fs from 'node:fs/promises'
import http from 'node:http'
import path from 'node:path'
import assert from 'node:assert/strict'
import {createHash} from 'node:crypto'
import {fileURLToPath,pathToFileURL} from 'node:url'
const root=process.env.LMM_ACCEPTANCE_ROOT
const round=process.env.LMM_ACCEPTANCE_ROUND
const suffix=process.env.LMM_ACCEPTANCE_ATTEMPT||''
assert.match(round||'',/^browser-[A-Za-z0-9._-]+$/)
assert.ok(!suffix||/^-[A-Za-z0-9._-]+$/.test(suffix))
const evidence=path.join(root,round)
const cfg=JSON.parse(await fs.readFile(path.join(evidence,'browser-private.json'),'utf8'))
const inputs=JSON.parse(await fs.readFile(path.join(evidence,'inputs.json'),'utf8'))
const ready=JSON.parse(await fs.readFile(path.join(evidence,'fixture-ready.json'),'utf8'))
assert.equal(cfg.evidence,evidence)
for(const key of ['feature_source_commit','core_source_commit','web_source_commit'])assert.equal(inputs[key],'fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5')
assert.equal(inputs.extore_source_commit,'cf3469dbe827afcfb2e3f054a63f7b9b7c40bc2b')
assert.equal(cfg.origin,'https://native.api.lmm.best')
assert.equal(cfg.extoreOrigin,'https://extore.lmm.best')
const proxy=new URL(cfg.proxy)
assert.equal(proxy.protocol,'http:');assert.equal(proxy.hostname,'127.0.0.1');assert.ok(proxy.port)
assert.equal(proxy.username,'');assert.equal(proxy.password,'')
const sha=bytes=>createHash('sha256').update(bytes).digest('hex')
for(const [pid,want] of [[ready.core_pid,inputs.core_sha256],[ready.go_pid,inputs.go_sha256],[ready.extore_pid,inputs.extore_sha256]]){
 assert.equal(sha(await fs.readFile('/proc/'+pid+'/exe')),want)
}
const out=path.join(evidence,'browser'+suffix);await fs.mkdir(out,{mode:0o700})
const ownSource=await fs.readFile(fileURLToPath(import.meta.url))
await fs.writeFile(path.join(out,'test-source.mjs'),ownSource,{mode:0o600})
const darkHelperPath=new URL('./dark-checks.mjs',import.meta.url)
const darkHelperSource=await fs.readFile(darkHelperPath)
await fs.writeFile(path.join(out,'dark-checks.mjs'),darkHelperSource,{mode:0o600})
const {verifyDark}=await import(darkHelperPath.href)
const provenance={inputs,dark_helper_sha256:sha(darkHelperSource),test_sha256:sha(ownSource),browser_hardware:'Chromium standard virtual WebAuthn authenticator; real Rust challenge/signature verification',counterexample_navigation:'pause only the genuine callback document request, test a different real Core session, then continue unchanged'}
const secrets=[cfg.adminPassword,cfg.controlKey,cfg.extoreSession]
const hide=value=>{if(typeof value==='string'&&value.length>12)secrets.push(value)}
const redact=text=>{
 for(const secret of secrets)if(secret)text=text.replaceAll(secret,'[PRIVATE]')
 return text.replace(/\bBearer [A-Za-z0-9._~-]+/g,'Bearer [PRIVATE]').replace(/lmmk_[A-Za-z0-9_-]+/g,'[PRIVATE_KEY]').replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g,'[PRIVATE_TOKEN]').replace(/([?&](?:code|state|code_challenge)=)[^&\s"\\]+/g,'$1[PRIVATE]')
}
const save=(name,value)=>fs.writeFile(path.join(out,name),redact(JSON.stringify(value,null,2))+'\n',{mode:0o600})
await save('provenance.json',provenance)
const {chromium}=await import(pathToFileURL(cfg.playwright).href)
const browser=await chromium.launch({headless:true,executablePath:cfg.chromium,
 proxy:{server:cfg.proxy},
 args:['--disable-dev-shm-usage','--no-sandbox','--ignore-certificate-errors-spki-list='+cfg.spki],
 env:{...process.env,TMPDIR:cfg.tmpdir}})
const context=await browser.newContext({viewport:{width:1440,height:1000},locale:'en-US',colorScheme:'light',proxy:{server:cfg.proxy},ignoreHTTPSErrors:false})
await context.addInitScript(()=>localStorage.setItem('i18nextLng','en'))
await context.addCookies([{name:'extore_session',value:cfg.extoreSession,url:cfg.extoreOrigin,httpOnly:true,secure:true,sameSite:'Lax'}])
const page=await context.newPage();page.setDefaultTimeout(20000)
const requests=[],errors=[],cases={},unconfiguredRequests=[],observedFailures=[],layouts=[]
const responseCaptures=[],activeStoreRequests=new Map(),requestMetadata=new WeakMap()
let requestSequence=0,lastStoreFinished=Date.now(),accessToken,activeConnection,continueCallbackNavigation=()=>{}
const runnerDeadline=Number(ready.deadline_epoch)-180
assert.ok(Number.isFinite(runnerDeadline)&&runnerDeadline-Date.now()/1000>1050,'fixture budget cannot cover real 900-second expiry plus cleanup')
const remainingSeconds=()=>runnerDeadline-Date.now()/1000
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms))
function checkBudget(label){assert.ok(remainingSeconds()>30,'browser budget exhausted: '+label)}
const deadlineTimer=setTimeout(()=>{void save('budget-exhausted.json',{time:Date.now()/1000,deadline:runnerDeadline});void browser.close()},Math.max(0,(runnerDeadline-Date.now()/1000)*1000))
deadlineTimer.unref()
page.on('pageerror',error=>errors.push(String(error.message).slice(0,1000)))
page.on('request',request=>{
 const url=new URL(request.url()),started=Date.now()/1000
 requestMetadata.set(request,{id:++requestSequence,request_started_at:started})
 if(url.origin===cfg.origin&&url.pathname.startsWith('/api/store/'))activeStoreRequests.set(request,{path:url.pathname,started})
 if(/^https?:$/.test(url.protocol)&&![cfg.origin,cfg.extoreOrigin].includes(url.origin))unconfiguredRequests.push({method:request.method(),origin:url.origin,path:url.pathname})
})
for(const event of ['requestfinished','requestfailed'])page.on(event,request=>{
 if(activeStoreRequests.delete(request))lastStoreFinished=Date.now()
})
page.on('response',response=>{
 const url=new URL(response.url())
 if(![cfg.origin,cfg.extoreOrigin].includes(url.origin))return
 const meta=requestMetadata.get(response.request())||{id:++requestSequence,request_started_at:response.request().timing().startTime/1000}
 meta.response_headers_at=Date.now()/1000
 requestMetadata.set(response.request(),meta)
 requests.push({...meta,origin:url.origin,method:response.request().method(),path:url.pathname,status:response.status(),retry_after:response.headers()['retry-after']??null})
 if(url.origin===cfg.origin&&['/api/user/login','/api/user/auth/refresh'].includes(url.pathname)&&response.status()===200){
  void response.json().then(body=>{if(body?.data?.access_token){accessToken=body.data.access_token;hide(accessToken)}}).catch(()=>{})
 }
 if(url.origin===cfg.extoreOrigin&&url.pathname.endsWith('/approve')&&response.status()===200){
  void response.json().then(body=>{if(body.redirect_uri){const u=new URL(body.redirect_uri);for(const k of ['code','state'])hide(u.searchParams.get(k))}}).catch(()=>{})
 }
 if(url.origin===cfg.origin&&url.pathname.startsWith('/api/store/')&&response.status()>=400){
  const capture=observeResponse('http-error-'+meta.id,response,true).then(observation=>{
   if(response.status()>=500)observedFailures.push({stage:'observed-http-error',...observation})
  }).catch(error=>save('http-error-'+meta.id+'-capture-failure.json',{...meta,path:url.pathname,status:response.status(),capture_error:String(error)}))
  responseCaptures.push(capture)
 }
})
async function uiBusy(){
 return {observed_at:Date.now()/1000,syncing_buttons:await page.getByRole('button',{name:'Syncing...',exact:true}).count().catch(()=>null),pending_store_requests:[...activeStoreRequests.values()],visible_connection:activeConnection?await activeConnection.isVisible().catch(()=>false):null}
}
async function observeResponse(name,response,withResources=false){
 const request=response.request(),u=new URL(response.url())
 const metadata=requestMetadata.get(request)||{request_started_at:request.timing().startTime/1000}
 let text,body,parse_error
 try{text=await response.text();try{body=JSON.parse(text)}catch(error){body=text;parse_error=String(error)}}catch(error){text=null;body=null;parse_error='body unavailable: '+String(error)}
 const observation={...metadata,method:request.method(),path:u.pathname,status:response.status(),response_body_received_at:Date.now()/1000,retry_after:response.headers()['retry-after']??null,body,...(parse_error?{parse_error}:{}),ui:await uiBusy()}
 await save(name+'.json',observation)
 if(withResources&&response.status()>=500){
  try{observation.resources=await resourceSnapshot('http-error-'+metadata.id);await save(name+'.json',observation)}
  catch(error){await save(name+'-resource-failure.json',{error:String(error)})}
 }
 return observation
}
async function settleStoreUI(connection){
 await connection.getByRole('button',{name:'Sync all products',exact:true}).waitFor({state:'visible'})
 await connection.getByRole('button',{name:'Sync all products',exact:true}).click({trial:true})
 const limit=Date.now()+20000
 while(activeStoreRequests.size||Date.now()-lastStoreFinished<400){
  assert.ok(Date.now()<limit,'store requests did not settle')
  await pause(100)
 }
 await Promise.allSettled([...responseCaptures])
}
const safeURL=()=>{const u=new URL(page.url());return u.origin+u.pathname}
async function capture(name){await page.screenshot({path:path.join(out,name+'.png'),fullPage:true,mask:[page.locator('input[type="password"]'),page.getByText(/lmmk_[A-Za-z0-9_-]+/)]})}
function responseAt(origin,pathname,method='GET'){
 const pending=page.waitForResponse(r=>new URL(r.url()).origin===origin&&new URL(r.url()).pathname===pathname&&r.request().method()===method)
 pending.catch(()=>{})
 return pending
}
async function localRequest(pathname,method='GET',body){
 assert.equal(new URL(page.url()).origin,cfg.origin)
 assert.ok(pathname.startsWith('/'))
 return page.evaluate(async({pathname,method,body,token})=>{
  const headers={'Authorization':'Bearer '+token}
  if(body!==undefined)headers['Content-Type']='application/json'
  const r=await fetch(pathname,{method,headers,body:body===undefined?undefined:JSON.stringify(body)})
  const text=await r.text();let value;try{value=JSON.parse(text)}catch{value=text}
  return{status:r.status,body:value,response_received_at:Date.now()/1000,retry_after:r.headers.get('retry-after')}
 },{pathname,method,body,token:accessToken})
}
async function privateControl(action,label){
 label+=suffix
 const endpoint=new URL('/__acceptance/control',cfg.controlOrigin)
 assert.equal(endpoint.protocol,'http:');assert.equal(endpoint.hostname,'127.0.0.1')
 const payload=Buffer.from(JSON.stringify({action,label}))
 const result=await new Promise((resolve,reject)=>{
  const request=http.request(endpoint,{method:'POST',headers:{'Content-Type':'application/json','Content-Length':payload.length,'X-Acceptance-Key':cfg.controlKey}},response=>{
   const chunks=[]
   response.on('data',chunk=>chunks.push(chunk))
   response.on('error',reject)
   response.on('end',()=>{try{resolve({status:response.statusCode,body:JSON.parse(Buffer.concat(chunks).toString('utf8'))})}catch(error){reject(error)}})
  })
  request.setTimeout(15000,()=>request.destroy(new Error('private control timeout')))
  request.on('error',reject);request.end(payload)
 })
 assert.equal(result.status,200);assert.equal(result.body.completed,true)
 return JSON.parse(await fs.readFile(path.join(evidence,(action==='snapshot'?'state-':'resources-')+label+'.json'),'utf8'))
}
async function snapshot(label){return privateControl('snapshot',label)}
async function resourceSnapshot(label){return privateControl('resources',label)}
function assertMapping(value,baseline){
 assert.equal(value.store.products.length,1);assert.equal(value.store.variants.length,2);assert.equal(value.store.sources.length,1)
 assert.equal(value.store.sources[0].id,baseline.store.sources[0].id)
 assert.equal(value.store.sources[0].product_id,baseline.store.sources[0].product_id)
 assert.deepEqual(value.store.sources[0].variant_ids,baseline.store.sources[0].variant_ids)
 assert.ok(value.store.sources[0].last_synced_at>=baseline.store.sources[0].last_synced_at)
 assert.ok(value.catalog_http_requests>baseline.catalog_http_requests);assert.ok(value.native_bridge.library_maps.length>0&&value.native_bridge.host_library_maps.length>0);assert.equal(value.native_bridge.go_exe_sha256,inputs.go_sha256)
 assert.equal(value.extore.issuances_count,0);assert.equal(value.store.orders_count,0);assert.equal(value.store.money_operations_count,0)
}
function assertCatalog(body){
 assert.equal(body.success,true);assert.equal(body.data.products.length,1)
 const product=body.data.products[0]
 assert.equal(product.id,cfg.extoreProduct)
 assert.deepEqual(product.variants.map(v=>v.id).sort(),['default','professional'])
 for(const [id,price,quota] of [['default','12.34',1000000],['professional','27.5',2000000]]){
  const variant=product.variants.find(v=>v.id===id)
  assert.equal(variant.price,price);assert.equal(variant.currency,'CNY');assert.equal(variant.marketplace.price_quota,quota)
 }
 assert.ok(!JSON.stringify(body).includes('PRIVATE_'))
}
function assertSync(observation){
 assert.equal(observation.status,200);assert.equal(observation.body.success,true)
 assert.equal(observation.body.data.results.length,1);assert.equal(observation.body.data.results[0].success,true)
 assert.equal(observation.body.data.results[0].source_id,cfg.extoreProduct)
}
function latestAccess(state){
 const rows=state.extore.token_expiry_metadata.filter(row=>row.kind==='access')
 assert.ok(rows.length>0,'real access-token expiry metadata missing')
 return rows.reduce((a,b)=>a.created>b.created?a:b)
}
async function requireHTTPStatus(name,response,status=200){
 if(response.status()!==status)await observeResponse(name+'-unexpected-response',response,true)
 assert.equal(response.status(),status)
}
function noPaidOrCodeIssuance(){
 assert.deepEqual(requests.filter(r=>r.method==='POST'&&(r.path.startsWith('/v1/')||r.path==='/api/integrations/commerce/cards')),[])
}
try{
 await page.goto(cfg.extoreOrigin+'/')
 await page.locator('#header-workspace').click()
 await page.locator('[data-edit="'+cfg.extoreProduct+'"]').click()
 await page.locator('#product-form').waitFor({state:'visible'})
 assert.equal(await page.locator('#v-id-0').inputValue(),'default')
 assert.equal(await page.locator('#v-id-1').inputValue(),'professional')
 assert.equal(await page.locator('#mp-price-0').inputValue(),'')
 assert.equal(await page.locator('#mp-price-1').inputValue(),'')
 for(const id of ['mp-price-0','mp-price-1']){
  const input=page.locator('#'+id)
  if(!await input.isVisible())await input.locator('xpath=ancestor::details[1]').locator('summary').first().click()
  await input.waitFor({state:'visible'})
 }
 await capture('extore-selling-prices-before')
 await page.locator('#mp-price-0').fill('1000000')
 await page.locator('#mp-price-1').fill('2000000')
 await capture('extore-selling-prices-configured')
 const savePending=responseAt(cfg.extoreOrigin,'/api/admin/products/'+cfg.extoreProduct,'PUT')
 await page.locator('#save-product').click()
 const saved=await savePending;await requireHTTPStatus('pricing-ui-save',saved)
 const actualRequest=saved.request().postDataJSON()
 const configured=actualRequest.variants.map(v=>({id:v.id,price:v.price,currency:v.currency,marketplace:v.marketplace}))
 assert.deepEqual(configured.map(v=>v.currency),['CNY','CNY'])
 assert.deepEqual(configured.map(v=>v.marketplace.price_quota),[1000000,2000000])
 await save('pricing-ui-save.json',{status:saved.status(),method:'PUT',path:'/api/admin/products/'+cfg.extoreProduct,variants:configured,meaning:'explicit synthetic selling Credits; no conversion from CNY reference price',response_keys:Object.keys(await saved.json())})
 await page.locator('[data-edit="'+cfg.extoreProduct+'"]').waitFor({state:'visible'})
 cases.real_extore_ui_configures_explicit_selling_prices='PASS'
 await capture('extore-products-after-save')
 const cdp=await context.newCDPSession(page)
 await cdp.send('WebAuthn.enable')
 await cdp.send('WebAuthn.addVirtualAuthenticator',{options:{protocol:'ctap2',transport:'internal',hasResidentKey:true,hasUserVerification:true,isUserVerified:true,automaticPresenceSimulation:true}})
 await page.goto(cfg.extoreOrigin+'/')
 const registration=await page.evaluate(async()=>{
  const read=async(path,body)=>{
   const r=await fetch(path,{method:body===undefined?'GET':'POST',headers:body===undefined?{}:{'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body)})
   return{status:r.status,body:await r.json()}
  }
  const status=await read('/api/auth/status')
  const option=await read('/api/auth/register/options',{})
  if(option.status!==200)return{status:status.status,options_status:option.status,error:option.body}
  const decode=value=>Uint8Array.from(atob(value.replace(/-/g,'+').replace(/_/g,'/')),c=>c.charCodeAt(0))
  const encode=value=>btoa(String.fromCharCode(...new Uint8Array(value))).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'')
  const publicKey=option.body
  publicKey.challenge=decode(publicKey.challenge)
  publicKey.user.id=decode(publicKey.user.id)
  publicKey.excludeCredentials?.forEach(v=>v.id=decode(v.id))
  const credential=await navigator.credentials.create({publicKey})
  const verified=await read('/api/auth/register/verify',{credential:{id:credential.id,rawId:encode(credential.rawId),type:credential.type,response:{clientDataJSON:encode(credential.response.clientDataJSON),attestationObject:encode(credential.response.attestationObject),transports:credential.response.getTransports?.()||[]},clientExtensionResults:credential.getClientExtensionResults()},name:'Independent browser synthetic authenticator'})
  return{status:status.status,options_status:option.status,verify_status:verified.status,verification:verified.body,authenticator_hardware:'virtual',signature_validation:'real Extore Rust WebAuthn'}
 })
 await save('passkey-registration.json',registration)
 assert.equal(registration.status,200);assert.equal(registration.options_status,200);assert.equal(registration.verify_status,200)
 cases.synthetic_passkey_real_registration='PASS'

 const navigationPending=responseAt(cfg.origin,'/api/site/navigation')
 await page.goto(cfg.origin+'/sign-in?redirect=%2Fstore%2Fmanage')
 await page.getByLabel('Username or Email',{exact:true}).waitFor({state:'visible'})
 await page.waitForFunction(()=>document.documentElement.classList.contains('dark')||document.documentElement.classList.contains('light'))
 const initialTheme=await page.evaluate(()=>({dark:document.documentElement.classList.contains('dark'),browser_color_scheme:'light',theme_cookie:document.cookie.match(/(?:^|; )vite-ui-theme=([^;]*)/)?.[1]??null}))
 await save('initial-lmm-theme.json',initialTheme)
 assert.equal(initialTheme.dark,true,'fixed candidate default theme is dark even when browser color scheme is light')
 cases.fresh_candidate_default_theme_dark='PASS'
 await page.getByLabel('Username or Email',{exact:true}).fill('consoleowner')
 await page.getByLabel('Password',{exact:true}).fill(cfg.adminPassword)
 const loginPending=responseAt(cfg.origin,'/api/user/login','POST')
 await page.getByRole('button',{name:'Sign in',exact:true}).click()
 const login=await loginPending;await requireHTTPStatus('primary-login',login)
 const loginBody=await login.json();assert.equal(loginBody.success,true);accessToken=loginBody.data.access_token;hide(accessToken)
 await page.waitForURL(u=>u.origin===cfg.origin&&u.pathname==='/store/manage')
 const identity=await localRequest('/core/v1/identity')
 assert.equal(identity.status,200);assert.equal(identity.body.user_id,cfg.owner_id);assert.equal(identity.body.platform_level,6)
 await save('core-identity.json',{user_id:identity.body.user_id,platform_level:identity.body.platform_level,credential_kind:identity.body.credential_kind})
 await page.getByRole('button',{name:'Connect Extore',exact:true}).waitFor({state:'visible'})
 const navigation=await observeResponse('public-navigation',await navigationPending)
 assert.equal(navigation.status,200);assert.ok(navigation.body&&typeof navigation.body==='object')
 assert.notEqual(navigation.body.success,false)
 cases.real_public_navigation_from_enabled_community='PASS'
 await snapshot('before-connect')
 cases.real_core_login='PASS'

 const secondContext=await browser.newContext({viewport:{width:1440,height:1000},locale:'en-US',colorScheme:'light',proxy:{server:cfg.proxy},ignoreHTTPSErrors:false})
 await secondContext.addInitScript(()=>localStorage.setItem('i18nextLng','en'))
 const secondPage=await secondContext.newPage();secondPage.setDefaultTimeout(20000)
 secondPage.on('request',request=>{
  requestMetadata.set(request,{id:++requestSequence,context:'secondary',request_started_at:Date.now()/1000})
 })
 secondPage.on('response',response=>{
  const u=new URL(response.url())
  if(u.origin!==cfg.origin)return
  const meta=requestMetadata.get(response.request())||{id:++requestSequence,context:'secondary',request_started_at:response.request().timing().startTime/1000}
  meta.response_headers_at=Date.now()/1000;requestMetadata.set(response.request(),meta)
  requests.push({...meta,origin:u.origin,method:response.request().method(),path:u.pathname,status:response.status(),retry_after:response.headers()['retry-after']??null})
  if(u.pathname.startsWith('/api/store/')&&response.status()>=400){
   const capture=observeResponse('secondary-http-error-'+meta.id,response,true).then(observation=>{
    if(response.status()>=500)observedFailures.push({stage:'observed-secondary-http-error',...observation})
   }).catch(error=>save('secondary-http-error-'+meta.id+'-capture-failure.json',{...meta,path:u.pathname,status:response.status(),capture_error:String(error)}))
   responseCaptures.push(capture)
  }
 })
 secondPage.on('pageerror',error=>errors.push('secondary: '+String(error.message).slice(0,1000)))
 await secondPage.goto(cfg.origin+'/sign-in?redirect=%2Fstore%2Fmanage')
 await secondPage.getByLabel('Username or Email',{exact:true}).fill('consoleowner')
 await secondPage.getByLabel('Password',{exact:true}).fill(cfg.adminPassword)
 const secondLoginPending=secondPage.waitForResponse(r=>new URL(r.url()).origin===cfg.origin&&new URL(r.url()).pathname==='/api/user/login'&&r.request().method()==='POST')
 secondLoginPending.catch(()=>{})
 await secondPage.getByRole('button',{name:'Sign in',exact:true}).click()
 const secondLogin=await secondLoginPending;await requireHTTPStatus('secondary-login',secondLogin)
 const secondLoginBody=await secondLogin.json();assert.equal(secondLoginBody.success,true)
 const secondAccessToken=secondLoginBody.data.access_token;hide(secondAccessToken)
 await secondPage.waitForURL(u=>u.origin===cfg.origin&&u.pathname==='/store/manage')
 const secondIdentity=await secondPage.evaluate(async token=>{
  const request_started_at=Date.now()/1000
  const r=await fetch('/core/v1/identity',{headers:{Authorization:'Bearer '+token}})
  const text=await r.text();let body;try{body=JSON.parse(text)}catch{body=text}
  return{context:'secondary',request_started_at,response_received_at:Date.now()/1000,status:r.status,retry_after:r.headers.get('retry-after'),body}
 },secondAccessToken)
 await save('secondary-core-identity.json',secondIdentity)
 assert.equal(secondIdentity.status,200);assert.equal(secondIdentity.body.user_id,identity.body.user_id)
 assert.equal(secondIdentity.body.credential_kind,identity.body.credential_kind)
 const cookiesA=await context.cookies(cfg.origin+'/api/user/auth/refresh'),cookiesB=await secondContext.cookies(cfg.origin+'/api/user/auth/refresh')
 const distinctLoginCookie=cookiesA.some(a=>a.httpOnly&&cookiesB.some(b=>b.httpOnly&&a.name===b.name&&a.domain===b.domain&&a.path===b.path&&a.value!==b.value))
 assert.equal(distinctLoginCookie,true);assert.notEqual(secondAccessToken,accessToken)
 await save('distinct-session-setup.json',{same_core_user:true,user_id:secondIdentity.body.user_id,credential_kind:secondIdentity.body.credential_kind,distinct_http_only_login_cookie:true,distinct_access_tokens:true,credentials_recorded:false,cookie_metadata: cookiesA.map(({name,path,httpOnly,secure,sameSite})=>({name,path,httpOnly,secure,sameSite}))})
 const authorizing=responseAt(cfg.origin,'/api/store/extore/authorize','POST')
 const authorizationRequest=page.waitForRequest(r=>{const u=new URL(r.url());return u.origin===cfg.extoreOrigin&&u.pathname==='/oauth/authorize'&&r.method()==='GET'})
 authorizationRequest.catch(()=>{})
 await page.getByRole('button',{name:'Connect Extore',exact:true}).click()
 const authResponse=await authorizing;await requireHTTPStatus('authorize-start',authResponse)
 const authURL=new URL((await authorizationRequest).url())
 assert.equal(authURL.origin,cfg.extoreOrigin);assert.equal(authURL.pathname,'/oauth/authorize')
 assert.equal(authURL.searchParams.get('redirect_uri'),cfg.origin+'/store/manage')
 for(const key of ['state','code_challenge'])hide(authURL.searchParams.get(key))
 assert.equal(authURL.searchParams.get('client_id'),'lmm-best-store')
 assert.deepEqual(authURL.searchParams.get('scope').split(' ').sort(),['marketplace.sync','products.read'])
 assert.equal(authURL.searchParams.get('code_challenge_method'),'S256')
 await save('authorize-contract.json',{status:200,origin:authURL.origin,path:authURL.pathname,client_id:'lmm-best-store',scope:['products.read','marketplace.sync'],pkce:'S256',redirect_uri:authURL.searchParams.get('redirect_uri'),evidence:'actual browser initial OAuth request after authorize HTTP 200'})
 await page.waitForURL(u=>u.origin===cfg.extoreOrigin&&u.pathname==='/connect/authorize')
 await page.locator('select[data-commerce-control][id$="-shop"]').selectOption(cfg.extoreShop)
 await page.locator('[data-commerce-product="'+cfg.extoreProduct+'"]').check()
 assert.equal(await page.locator('[data-commerce-scope="cards.issue"]').count(),0)
 assert.equal(await page.locator('[data-commerce-scope="products.read"]').isChecked(),true)
 assert.equal(await page.locator('[data-commerce-scope="marketplace.sync"]').isChecked(),true)
 const expiry=await page.evaluate(()=>{const d=new Date(Date.now()+3600_000);return{local:new Date(d.getTime()-d.getTimezoneOffset()*60000).toISOString().slice(0,16),timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,offset_minutes:d.getTimezoneOffset(),now_epoch:Date.now(),target_epoch:d.getTime()}})
 await save('expiry-input.json',expiry)
 const expires=expiry.local
 await page.locator('input[type="datetime-local"]').fill(expires)
 await page.locator('button[data-commerce-control][id$="-review"]').click()
 await page.getByText('Confirm this authorization',{exact:true}).waitFor({state:'visible'})
 await capture('extore-consent-review')

 const navigationGate=new Promise(resolve=>{continueCallbackNavigation=resolve})
 const callbackPath=cfg.origin+'/store/manage?**'
 const approvedCallback=page.waitForRequest(request=>{const u=new URL(request.url());return request.isNavigationRequest()&&u.origin===cfg.origin&&u.pathname==='/store/manage'&&u.searchParams.has('code')&&u.searchParams.has('state')&&u.searchParams.has('iss')})
 approvedCallback.catch(()=>{})
 await page.route(callbackPath,async route=>{
  const request=route.request(),u=new URL(request.url())
  if(request.isNavigationRequest()&&u.searchParams.has('code')&&u.searchParams.has('state')&&u.searchParams.has('iss'))await navigationGate
  await route.continue()
 })
 const reauthPending=responseAt(cfg.extoreOrigin,'/api/auth/reauth/passkey/verify','POST')
 const approvalPending=page.waitForResponse(r=>new URL(r.url()).origin===cfg.extoreOrigin&&/^\/api\/admin\/commerce\/requests\/[^/]+\/approve$/.test(new URL(r.url()).pathname)&&r.request().method()==='POST')
 approvalPending.catch(()=>{})
 const importedPending=responseAt(cfg.origin,'/api/store/extore/catalog','POST')
 await page.locator('button[data-commerce-control][id$="-approve"]').click()
 await capture('extore-reauthentication-prompt')
 await page.getByRole('button',{name:/Passkey/}).last().click()
 await requireHTTPStatus('passkey-reauth',await reauthPending)
 const approved=await approvalPending;await requireHTTPStatus('consent-approval',approved)
 cases.real_consent_and_passkey_reauthentication='PASS'

 const mainRequestsAfterApproval=requests.length
 try{
  const approvedRequest=await approvedCallback
  const actualCallback=approvedRequest.url(),callback=new URL(actualCallback)
  hide(callback.searchParams.get('code'));hide(callback.searchParams.get('state'))
  const denied=await secondPage.evaluate(async({token,callback_url})=>{
   const request_started_at=Date.now()/1000
   const r=await fetch('/api/store/extore/catalog',{method:'POST',headers:{Authorization:'Bearer '+token,'Content-Type':'application/json'},body:JSON.stringify({callback_url})})
   const text=await r.text();let body;try{body=JSON.parse(text)}catch{body=text}
   return{context:'secondary',request_started_at,response_received_at:Date.now()/1000,status:r.status,retry_after:r.headers.get('retry-after'),body}
  },{token:secondAccessToken,callback_url:actualCallback})
  await save('cross-session-denial.json',{...denied,same_core_user:true,distinct_http_only_login_cookie:distinctLoginCookie,callback_source:'actual approved Extore browser navigation; request paused without changing bytes'})
  assert.equal(denied.status,409);assert.equal(denied.body.code,'STORE_CONFLICT');assert.equal(denied.body.success,false)
  const isolated=await snapshot('cross-session-denied')
  assert.equal(isolated.store.flows.length,1);assert.equal(isolated.store.flows[0].consumed,false)
  assert.equal(isolated.store.flows[0].user_id,cfg.owner_id)
  assert.equal(isolated.store.connections.length,0);assert.equal(isolated.store.products.length,0)
  assert.equal(isolated.extore.grants.length,1);assert.equal(isolated.extore.grants[0].code_used,null)
  assert.equal(isolated.extore.token_counts.length,0)
  assert.ok(!isolated.upstream_http.some(e=>e.path==='/api/integrations/commerce/token'))
  cases.same_user_different_session_cannot_consume_flow='PASS'
 }finally{
  continueCallbackNavigation()
 }
 const importedResponse=await importedPending
 const importObservation=await observeResponse('import-response',importedResponse)
 const importedBody=importObservation.body
 assert.equal(importObservation.status,200)
 assert.equal(importedBody.success,true)
 assert.ok(requests.slice(mainRequestsAfterApproval).some(r=>r.context!=='secondary'&&r.origin===cfg.origin&&r.method==='POST'&&r.path==='/api/user/auth/refresh'&&r.status===200))
 cases.same_session_refresh_survives_callback='PASS'
 await secondContext.close()
 await page.unroute(callbackPath)
 const imported=importedBody.data;assert.ok(imported.connection.id);assert.equal(imported.results.length,1);assert.equal(imported.results[0].success,true)
 assert.equal(imported.results[0].source_id,cfg.extoreProduct)
 assert.ok(!JSON.stringify(imported).includes('PRIVATE_'))
 await page.waitForURL(u=>u.origin===cfg.origin&&u.pathname==='/store/manage')
 await capture('lmm-import-result')
 await page.getByRole('button',{name:'Back to store',exact:true}).click()
 assert.ok(imported.results[0].product_id)
 await page.locator('[data-store-product-id='+JSON.stringify(String(imported.results[0].product_id))+']').waitFor({state:'visible'})
 const connection=page.getByRole('region',{name:'Fixture '+cfg.extoreShop,exact:true});activeConnection=connection
 await connection.waitFor({state:'visible'})
 const catalogPending=responseAt(cfg.origin,'/api/store/extore/connections/'+imported.connection.id+'/catalog')
 await connection.getByRole('button',{name:'View products',exact:true}).click()
 const catalogObservation=await observeResponse('catalog-observation',await catalogPending)
 assert.equal(catalogObservation.status,200);assertCatalog(catalogObservation.body)
 await save('catalog-response.json',catalogObservation.body)
 const importedState=await snapshot('imported')
 assert.equal(importedState.store.flows.length,1);assert.equal(importedState.store.flows[0].consumed,true)
 assert.equal(importedState.store.sources.length,1);assert.equal(importedState.store.products.length,1);assert.equal(importedState.store.variants.length,2)
 assert.deepEqual(Object.keys(importedState.store.sources[0].variant_ids).sort(),['default','professional'])
 assert.equal(importedState.store.connections[0].credential_is_encrypted,true)
 assert.ok(importedState.catalog_http_requests>0)
 assert.ok(importedState.native_bridge.library_maps.length>0&&importedState.native_bridge.host_library_maps.length>0)
 assert.equal(importedState.native_bridge.go_exe_sha256,inputs.go_sha256)
 assert.equal(importedState.extore.issuances_count,0)
 cases.real_native_catalog_and_product_mapping='PASS'
 const catalogPath='/api/store/extore/connections/'+imported.connection.id+'/catalog'
 const syncPath='/api/store/extore/connections/'+imported.connection.id+'/sync-all'
 await settleStoreUI(connection)
 await resourceSnapshot('before-normal-syncs')
 for(let index=1;index<=2;index++){
  checkBudget('normal-sync-'+index)
  const requestsBefore=requests.length
  const syncPending=responseAt(cfg.origin,syncPath,'POST')
  const refreshedCatalogPending=responseAt(cfg.origin,catalogPath)
  await connection.getByRole('button',{name:'Sync all products',exact:true}).click()
  const syncObservation=await observeResponse('normal-sync-'+index,await syncPending)
  const catalogObservation=await observeResponse('normal-sync-'+index+'-catalog',await refreshedCatalogPending)
  assertSync(syncObservation);assert.equal(catalogObservation.status,200);assertCatalog(catalogObservation.body)
  await settleStoreUI(connection)
  const syncedState=await snapshot('synchronized-'+index)
  assertMapping(syncedState,importedState)
  const unexpected=requests.slice(requestsBefore).filter(r=>r.origin===cfg.origin&&r.path.startsWith('/api/store/')&&r.status>=500)
  await save('normal-sync-'+index+'-settled.json',{ui:await uiBusy(),unexpected_server_errors:unexpected,source:syncedState.store.sources[0],mapping_unchanged:true})
  assert.deepEqual(unexpected,[])
  cases['real_ui_native_sync_'+index+'_preserves_mapping']='PASS'
  console.log(JSON.stringify({phase:'normal-sync-'+index,overall:'PASS'}))
 }
 await capture('lmm-native-catalog-desktop')
 await page.setViewportSize({width:390,height:844})
 await connection.scrollIntoViewIfNeeded()
 const layout=await page.evaluate(()=>({viewport:innerWidth,width:document.documentElement.scrollWidth,client:document.documentElement.clientWidth,dark:document.documentElement.classList.contains('dark')}))
 assert.equal(layout.viewport,390);assert.ok(layout.width<=layout.client+1)
 await capture('lmm-native-catalog-mobile');await save('mobile-layout.json',layout)
 await page.setViewportSize({width:1440,height:1000})

 // A separate real interaction sequence; preserve its failure even if later cases complete.
 await connection.getByRole('button',{name:'Hide products',exact:true}).click()
 await settleStoreUI(connection)
 await resourceSnapshot('before-overlap')
 const overlapStart=Date.now()/1000
 try{
  const overlapSyncPending=responseAt(cfg.origin,syncPath,'POST')
  const overlapCatalogPending=responseAt(cfg.origin,catalogPath)
  await connection.getByRole('button',{name:'Sync all products',exact:true}).click()
  const beforeView=await uiBusy()
  await connection.getByRole('button',{name:'View products',exact:true}).click()
  const afterView=await uiBusy()
  await save('overlap-actions.json',{started_at:overlapStart,before_view:beforeView,after_view:afterView})
  await resourceSnapshot('during-overlap')
  const outcomes=await Promise.allSettled([
   overlapSyncPending.then(response=>observeResponse('overlap-sync',response)),
   overlapCatalogPending.then(response=>observeResponse('overlap-catalog',response))
  ])
  await save('overlap-outcomes.json',outcomes.map(item=>item.status==='fulfilled'?{status:item.status,observation:item.value}:{status:item.status,error:String(item.reason)}))
  assert.equal(outcomes[0].status,'fulfilled');assert.equal(outcomes[1].status,'fulfilled')
  assertSync(outcomes[0].value);assert.equal(outcomes[1].value.status,200);assertCatalog(outcomes[1].value.body)
  assert.ok(beforeView.syncing_buttons>0,'View action did not occur while actual UI was syncing; overlap coverage not observed')
  await settleStoreUI(connection)
  const afterOverlap=await snapshot('after-overlap')
  assertMapping(afterOverlap,importedState)
  cases.real_ui_overlapping_sync_and_view='PASS'
 }catch(error){
  cases.real_ui_overlapping_sync_and_view='FAIL'
  observedFailures.push({stage:'overlapping-sync-and-view',time:Date.now()/1000,error:String(error)})
  await save('overlap-failure.json',{error:String(error),ui:await uiBusy(),observed_failures:observedFailures})
  await capture('overlap-failure').catch(()=>{})
  await resourceSnapshot('after-overlap-failure').catch(()=>{})
 }
 await settleStoreUI(connection)
 const hideProducts=connection.getByRole('button',{name:'Hide products',exact:true})
 if(await hideProducts.isVisible())await hideProducts.click()
 await settleStoreUI(connection)
 const beforeWait=await snapshot('before-access-expiry')
 const accessBefore=latestAccess(beforeWait)
 const expiresAt=Number(accessBefore.expires),resumeAt=expiresAt+3
 assert.ok(Number.isFinite(expiresAt)&&Math.abs(expiresAt-Number(accessBefore.created)-900)<0.01,'unexpected real Extore access-token lifetime')
 assert.ok(resumeAt+180<runnerDeadline,'remaining fixture budget cannot cover actual access expiry and final UI checks')
 await save('expiry-wait-plan.json',{started_at:Date.now()/1000,access_metadata:accessBefore,public_connection_expires_at:imported.connection.expires_at,resume_after_epoch:resumeAt,browser_deadline_epoch:runnerDeadline,fixture_deadline_epoch:ready.deadline_epoch,method:'wait for real issued access-token expiry; no DB or clock mutation; catalog hidden through real UI'})
 let heartbeat=0
 while(Date.now()/1000<resumeAt){
  checkBudget('real-access-expiry-wait')
  const resource=await resourceSnapshot('expiry-wait-'+heartbeat)
  assert.ok(resource.processes.every(process=>process.alive),'owned fixture process exited during real expiry wait')
  const progress={phase:'waiting-for-real-access-expiry',heartbeat:heartbeat++,observed_at:Date.now()/1000,resume_at:resumeAt,remaining_wait_seconds:Math.max(0,resumeAt-Date.now()/1000),remaining_run_seconds:remainingSeconds(),processes_alive:true}
  await fs.appendFile(path.join(out,'expiry-wait-journal.jsonl'),JSON.stringify(progress)+'\n',{mode:0o600})
  console.log(JSON.stringify(progress))
  await pause(Math.min(30000,Math.max(0,(resumeAt-Date.now()/1000)*1000)))
 }
 const expiredState=await snapshot('expired-before-view')
 const expiredAccess=latestAccess(expiredState)
 assert.equal(expiredAccess.created,accessBefore.created);assert.equal(expiredAccess.expires,accessBefore.expires)
 assert.ok(expiredState.time>Number(expiredAccess.expires),'access token is not actually expired')
 const expiryViewPending=responseAt(cfg.origin,catalogPath)
 const expiryViewStarted=Date.now()/1000
 await connection.getByRole('button',{name:'View products',exact:true}).click()
 const afterExpiry=await observeResponse('catalog-after-real-access-expiry',await expiryViewPending)
 assert.equal(afterExpiry.status,200);assertCatalog(afterExpiry.body)
 await settleStoreUI(connection)
 const refreshedState=await snapshot('after-access-refresh')
 assertMapping(refreshedState,importedState)
 const newAccess=latestAccess(refreshedState)
 assert.ok(Number(newAccess.created)>Number(expiredAccess.expires))
 assert.ok(Number(newAccess.expires)>refreshedState.time)
 const oldTokenExchanges=expiredState.upstream_http.filter(e=>e.path==='/api/integrations/commerce/token'&&e.method==='POST'&&e.status===200).length
 const newTokenExchanges=refreshedState.upstream_http.filter(e=>e.path==='/api/integrations/commerce/token'&&e.method==='POST'&&e.status===200).length
 assert.equal(newTokenExchanges,oldTokenExchanges+1)
 const oldRefreshUsed=expiredState.extore.token_expiry_metadata.filter(row=>row.kind==='refresh'&&row.used).length
 const newRefreshUsed=refreshedState.extore.token_expiry_metadata.filter(row=>row.kind==='refresh'&&row.used).length
 assert.equal(newRefreshUsed,oldRefreshUsed+1)
 assert.equal(refreshedState.extore.token_expiry_metadata.filter(row=>row.kind==='access').length,expiredState.extore.token_expiry_metadata.filter(row=>row.kind==='access').length+1)
 assert.ok(Number(refreshedState.store.connections[0].version)>Number(expiredState.store.connections[0].version))
 await save('actual-access-expiry-refresh-proof.json',{before_access:expiredAccess,after_access:newAccess,view_started_at:expiryViewStarted,catalog_status:afterExpiry.status,token_http_200_before:oldTokenExchanges,token_http_200_after:newTokenExchanges,refresh_used_before:oldRefreshUsed,refresh_used_after:newRefreshUsed,connection_version_before:expiredState.store.connections[0].version,connection_version_after:refreshedState.store.connections[0].version,mapping_unchanged:true,catalog_http_requests:refreshedState.catalog_http_requests,native_library_mapped:refreshedState.native_bridge.library_maps.length>0&&refreshedState.native_bridge.host_library_maps.length>0})
 cases.real_expired_extore_access_token_refresh_preserves_mapping='PASS'
 console.log(JSON.stringify({phase:'real-access-expiry-refresh',overall:'PASS'}))
 await capture('lmm-catalog-after-real-access-refresh')
 checkBudget('disconnect-and-dark')

 await connection.getByRole('button',{name:'Disconnect',exact:true}).click()
 const dialog=page.getByRole('alertdialog')
 await dialog.getByText('Disconnect Extore?',{exact:true}).waitFor({state:'visible'})
 const disconnectedPending=responseAt(cfg.origin,'/api/store/extore/connections/'+imported.connection.id+'/disconnect','POST')
 await dialog.getByRole('button',{name:'Disconnect',exact:true}).click()
 const disconnected=await observeResponse('disconnect-response',await disconnectedPending)
 assert.equal(disconnected.status,200);assert.equal(disconnected.body.success,true)
 await connection.waitFor({state:'hidden'})
 const rejected=await localRequest('/api/store/extore/connections/'+imported.connection.id+'/catalog')
 await save('after-revoke-denied.json',rejected)
 assert.equal(rejected.status,409);assert.ok(JSON.stringify(rejected.body).includes('STORE_EXTORE_REAUTHORIZE'))
 const final=await snapshot('disconnected')
 assert.equal(final.store.connections.length,1);assert.ok(Number(final.store.connections[0].revoked_at)>0)
 assert.equal(final.extore.grants.length,1);assert.equal(final.extore.grants[0].revoked,1)
 assert.equal(final.store.sources.length,1);assert.equal(final.store.products.length,1);assert.equal(final.store.variants.length,2)
 assert.equal(final.store.sources[0].product_id,importedState.store.sources[0].product_id)
 assert.equal(final.extore.issuances_count,0);assert.equal(final.store.orders_count,0);assert.equal(final.store.money_operations_count,0)
 assert.equal(final.catalog_http_requests,refreshedState.catalog_http_requests,'revoked local connection must not issue another native HTTPS catalog request')
 assert.ok(final.upstream_http.some(e=>e.path==='/api/integrations/commerce/revoke'&&e.method==='POST'&&e.status===200))
 cases.real_ui_disconnect_and_denial='PASS'
 Object.assign(cases,await verifyDark({page,productID:importedState.store.products[0].id,capture,save}))
 await capture('lmm-disconnected-imports-retained')
 noPaidOrCodeIssuance();assert.deepEqual(errors,[])
 await Promise.allSettled([...responseCaptures])
 await save('observed-failures.json',observedFailures)
 assert.deepEqual(observedFailures,[],'one or more independent HTTP/overlap failures occurred; later operations do not erase them')
 await fs.writeFile(path.join(evidence,'browser'+suffix+'-state-private.json'),JSON.stringify(await context.storageState()),{mode:0o600})
 await save('result.json',{provenance,overall:'PASS',cases,requests,page_errors:errors,unconfigured_requests_blocked_by_closed_proxy:unconfiguredRequests})
 console.log(JSON.stringify({overall:'PASS',cases,evidence:out}))
}catch(error){
 await capture('failure').catch(()=>{})
 await save('failure-text.json',{url:safeURL(),text:await page.locator('body').innerText().catch(()=>'')})
 await save('result.json',{provenance,overall:'FAIL',cases,error:String(error),error_stack:String(error.stack).slice(0,4000),observed_failures:observedFailures,requests,page_errors:errors,unconfigured_requests_blocked_by_closed_proxy:unconfiguredRequests})
 console.log(JSON.stringify({overall:'FAIL',cases,error:redact(String(error)).slice(0,1200),evidence:out}));process.exitCode=1
}finally{clearTimeout(deadlineTimer);continueCallbackNavigation();await Promise.allSettled(responseCaptures);await browser.close()}
