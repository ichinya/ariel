// Real Pi/Docker/API conformance against a deterministic local model fixture.
// No paid inference, host Pi authorization or source-repository mutations.
import {createServer,request as httpRequest} from 'node:http';
import {spawn,spawnSync} from 'node:child_process';
import {readFileSync,writeFileSync,mkdirSync,statSync,existsSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {randomUUID} from 'node:crypto';
import assert from 'node:assert/strict';
const binary=resolve(process.env.ARIEL_BIN||'target/debug/ariel.exe');
const primary=JSON.parse(readFileSync('.ariel/config.json'));
const run=resolve('test-results','fixture-'+randomUUID()),data=join(run,'data');mkdirSync(run,{recursive:true});
const checks=[],sleep=ms=>new Promise(r=>setTimeout(r,ms)),terminal=s=>['succeeded','failed','cancelled','interrupted'].includes(s);
let daemon,config,base,owner,client,requests=0;
function cmd(program,args){const r=spawnSync(program,args,{encoding:'utf8',windowsHide:true});assert.equal(r.status,0,program+' failed');return r.stdout.trim();}
const text=content=>typeof content==='string'?content:(content||[]).filter(v=>v.type==='text').map(v=>v.text).join('');
const fixture=createServer(async(req,res)=>{
 if(req.url!== '/v1/chat/completions'){res.writeHead(404).end();return;}
 let raw='';for await(const chunk of req)raw+=chunk;requests++;
 const body=JSON.parse(raw),messages=body.messages;
 const latest=[...messages].reverse().find(m=>m.role==='user'&&text(m.content).startsWith('ARIEL_FIXTURE '));
 const spec=latest?JSON.parse(text(latest.content).slice(14)):{final:'fixture ready'};
 if(spec.error){res.writeHead(400,{'Content-Type':'application/json'}).end(JSON.stringify({error:{message:'fixture_model_error',type:'invalid_request_error'}}));return;}
 res.writeHead(200,{'Content-Type':'text/event-stream','Cache-Control':'no-cache'});
 const chunk=(delta,reason=null)=>res.write('data: '+JSON.stringify({id:'chatcmpl-fixture',object:'chat.completion.chunk',created:1,model:'fixture',choices:[{index:0,delta,finish_reason:reason}]})+'\n\n');
 chunk({role:'assistant'});
 if(spec.tool&&messages.at(-1).role!=='tool'){
  chunk({tool_calls:[{index:0,id:'call_'+randomUUID(),type:'function',function:{name:spec.tool,arguments:JSON.stringify(spec.args)}}]});chunk({},'tool_calls');
 }else{
  const toolResult=messages.at(-1).role==='tool'?text(messages.at(-1).content):'';
  const result=spec.echo?toolResult:spec.final||'FIXTURE_OK';for(let i=0;i<result.length;i+=5)chunk({content:result.slice(i,i+5)});chunk({},'stop');
 }
 res.end('data: [DONE]\n\n');
});
await new Promise(r=>fixture.listen(0,'0.0.0.0',r));
const fixturePort=fixture.address().port;
const spare=async()=>{const s=createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p;};
async function start(){
 daemon=spawn(binary,['start','--data',data],{windowsHide:true,stdio:['ignore','pipe','pipe']});let errors='';daemon.stderr.on('data',b=>errors+=b);daemon.stdout.resume();
 for(let i=0;i<100;i++){try{const r=await fetch(base+'/health');if(r.ok)return;}catch{}if(daemon.exitCode!==null)throw Error('Fixture daemon exited: '+errors);await sleep(100);}
 throw Error('Fixture daemon did not start');
}
async function api(path,method='GET',body,token=client,status=200,headers={}){
 const r=await fetch(base+path,{method,headers:{Authorization:'Bearer '+token,...(body?{'Content-Type':'application/json'}:{}),...headers},body:body?JSON.stringify(body):undefined});
 const value=await r.json();assert.equal(r.status,status,JSON.stringify(value));return value;
}
async function waitJob(id,wanted=terminal){for(let i=0;i<240;i++){const j=await api('/v1/jobs/'+id);if(wanted(j.state))return j;await sleep(250);}throw Error('Job timeout '+id);}
async function submit(session,spec){return api('/v1/jobs','POST',{session_id:session.id,input:'ARIEL_FIXTURE '+JSON.stringify(spec),idempotency_key:randomUUID(),timeout_seconds:60});}
async function success(session,spec){const j=await submit(session,spec),done=await waitJob(j.id);assert.equal(done.state,'succeeded',done.error);return done;}
async function session(workspace){return api('/v1/sessions','POST',{agent_id:'pi',provider_id:'ariel-fixture',model_id:'fixture',workspace_id:workspace.id});}
async function events(job){const r=await fetch(base+'/v1/jobs/'+job.id+'/events?after=0',{headers:{Authorization:'Bearer '+client}});return(await r.text()).split('\n').filter(l=>l.startsWith('data:')).map(l=>JSON.parse(l.slice(5)));}
async function question(job){for(let i=0;i<200;i++){const q=(await api('/v1/questions','GET',null,owner)).questions.find(q=>q.job_id===job.id);if(q)return q;const j=await api('/v1/jobs/'+job.id);if(terminal(j.state))throw Error('Expected question, got '+j.state+': '+j.error);await sleep(250);}throw Error('Question timeout');}
async function noContainer(sessionId){const value=cmd('docker',['ps','-aq','--filter','label=ariel.session='+sessionId]);assert.equal(value,'');}
async function stop(){if(daemon?.exitCode===null){await api('/v1/admin/shutdown','POST',{},owner);await new Promise((r,j)=>{const timeout=setTimeout(()=>j(Error('Shutdown timeout')),45000);daemon.once('exit',()=>{clearTimeout(timeout);r();});});}}
try{
 cmd(binary,['init','--data',data]);config=JSON.parse(readFileSync(join(data,'config.json')));
 config.image_id=primary.image_id;config.bind='127.0.0.1:'+await spare();config.test_bind='127.0.0.1:'+await spare();config.clients.push({id:'other',token:randomUUID()+randomUUID()});
 writeFileSync(join(data,'config.json'),JSON.stringify(config));base='http://'+config.bind;owner=config.owner_token;client=config.clients[0].token;
 writeFileSync(join(data,'profile/auth.json'),JSON.stringify({'ariel-fixture':{type:'api_key',key:'fixture-secret-for-redaction'}}));
 writeFileSync(join(data,'profile/models.json'),JSON.stringify({providers:{'ariel-fixture':{baseUrl:'http://host.docker.internal:'+fixturePort+'/v1',api:'openai-completions',models:[{id:'fixture',name:'Ariel fixture',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:16384,maxTokens:2048}]}}}));
 await start();
 const w=await api('/v1/workspaces','POST',{mode:'folder'});
 await api('/v1/sessions','POST',{provider_id:'ariel-fixture',model_id:'fixture',workspace_id:w.id},client,409);
 const before=(await api('/v1/agents/pi/models')).models;assert.equal(before.length,0);
 const catalog=await api('/v1/admin/catalog','GET',null,owner),model=catalog.catalog.models.find(m=>m.provider_id==='ariel-fixture');assert(model?.configured);
 await api('/v1/admin/providers/ariel-fixture','PUT',{enabled:true},owner);await api('/v1/admin/models/'+model.selection_id,'PUT',{enabled:true},owner);
 assert.equal((await api('/v1/agents/pi/models')).models.length,1);checks.push('disabled by default; exact provider/model enablement');
 const s=await session(w);
 await api('/v1/sessions','POST',{provider_id:'ariel-fixture',model_id:'fixture',workspace_id:w.id,required_capabilities:['egress_allowlist']},client,409);
 await api('/v1/workspaces/'+w.id+'/files','GET',null,config.clients[1].token,404);
 await api('/v1/admin/catalog','GET',null,client,403);
 await api('/v1/agents','GET',null,client,403,{Origin:'https://untrusted.invalid'});
 await new Promise((resolve,reject)=>{const request=httpRequest(base+'/v1/agents',{headers:{Authorization:'Bearer '+client,Host:'untrusted.invalid'}},response=>{response.resume();try{assert.equal(response.statusCode,403);resolve();}catch(e){reject(e);}});request.on('error',reject);request.end();});
 checks.push('client ownership, owner role, Origin/Host and strict unsupported capability rejection');
 const first=await success(s,{tool:'write',args:{path:'fixture.txt',content:'FIRST\n'},final:'FIRST_OK'});
 assert.equal((await api('/v1/workspaces/'+w.id+'/file?path=fixture.txt')).text,'FIRST\n');assert(first.sandbox_receipt?.non_root&&first.sandbox_receipt?.read_only_image);
 const second=await success(s,{tool:'read',args:{path:'fixture.txt'},echo:true});assert(second.result.includes('FIRST'));
 const replay=await events(first);assert.equal(replay.filter(e=>e.kind==='terminal').length,1);assert(replay.some(e=>e.kind==='tool_end'));
 const tail=await fetch(base+'/v1/jobs/'+first.id+'/events?after='+replay.at(-1).cursor,{headers:{Authorization:'Bearer '+client}});assert.equal((await tail.text()).trim(),'');
 await noContainer(s.id);checks.push('real Pi tools, session history, durable SSE cursor and confirmed container stop');
 const escaped=await success(s,{tool:'bash',args:{command:'test ! -e /var/run/docker.sock && test ! -e /host && test ! -e /workspace/../'+w.id+' && ! touch /etc/ariel-denied && echo ISOLATED'},echo:true});assert(escaped.result.includes('ISOLATED'));
 await api('/v1/workspaces/'+w.id+'/file?path=../../config.json','GET',null,client,409);
 const redacted=await success(s,{tool:'bash',args:{command:"cat /state/agent/auth.json"},echo:true});assert(redacted.result.includes('[REDACTED]'));const redactedEvents=await events(redacted);assert(!JSON.stringify(redactedEvents).includes('fixture-secret-for-redaction'));assert(!redactedEvents.filter(e=>e.kind==='text').map(e=>e.data.delta).join('').includes('fixture-secret-for-redaction'));
 checks.push('host/socket/sibling boundary, read-only image, artifact traversal rejection and known credential redaction');
 const failed=await submit(s,{error:true});assert.equal((await waitJob(failed.id)).state,'failed');checks.push('model errors never succeed');
 const busy=await submit(s,{tool:'bash',args:{command:'while true; do echo tick >> counter.txt; sleep 0.2; done'},final:'must not finish'});
 await waitToolStart(busy,'counter.txt');
 const body={session_id:s.id,input:busy.input,idempotency_key:busy.idempotency_key,timeout_seconds:60};
 assert.equal((await api('/v1/jobs','POST',body)).id,busy.id);await api('/v1/jobs','POST',{...body,input:'different'},client,409);
 await api('/v1/jobs','POST',{...body,idempotency_key:randomUUID()},client,409);
 await api('/v1/jobs/'+busy.id+'/cancel','POST',{});assert.equal((await waitJob(busy.id)).state,'cancelled');await noContainer(s.id);
 const counter=join(data,'workspaces',w.id,'tree/counter.txt');const bytes=statSync(counter).size;await sleep(700);assert.equal(statSync(counter).size,bytes);
 checks.push('idempotency, serialized dispatch, cancel stops shell descendants');
 const source=join(run,'source');mkdirSync(source);cmd('git',['init',source]);writeFileSync(join(source,'tracked.txt'),'source unchanged\n');cmd('git',['-C',source,'add','tracked.txt']);cmd('git',['-C',source,'-c','user.name=Ariel fixture','-c','user.email=fixture@example.invalid','commit','-m','fixture']);const sha=cmd('git',['-C',source,'rev-parse','HEAD']);writeFileSync(join(source,'tracked.txt'),'dirty original preserved\n');
 const wt=await api('/v1/workspaces','POST',{mode:'worktree',source_path:source,source_revision:sha},owner);assert.equal(wt.source_revision,sha);
 const ws=await session(wt),git=await success(ws,{tool:'bash',args:{command:"git -c safe.directory=/workspace/tree rev-parse HEAD && cat tracked.txt && printf 'changed\\n' > tracked.txt && git -c safe.directory=/workspace/tree diff --stat"},echo:true});assert(git.result.includes(sha));assert(git.result.includes('source unchanged'));assert(git.result.includes('tracked.txt'));assert.equal(readFileSync(join(source,'tracked.txt'),'utf8'),'dirty original preserved\n');
 checks.push('owned real worktree: exact HEAD, writable Git metadata, original source preserved');
 const external=join(run,'external');mkdirSync(external);writeFileSync(join(external,'allowed.txt'),'READ_ONLY_GRANTED\n');
 const denied=await submit(s,{tool:'ariel_request_path',args:{path:external,access:'read'},final:'DENIED_DONE'}),dq=await question(denied);
 await api('/v1/access-requests/'+dq.id+'/decision','POST',{request_digest:dq.request_digest,decision:'allowed'},client,403);
 await api('/v1/access-requests/'+dq.id+'/decision','POST',{request_digest:'stale',decision:'allowed'},owner,409);
 await api('/v1/access-requests/'+dq.id+'/decision','POST',{request_digest:dq.request_digest,decision:'denied'},owner);assert.equal((await waitJob(denied.id)).state,'succeeded');assert.equal((await api('/v1/workspaces')).workspaces.find(v=>v.id===w.id).grants.length,0);
 const request=await submit(s,{tool:'ariel_request_path',args:{path:external,access:'read'},final:'MUST_STOP'}),q=await question(request);
 const grant=await api('/v1/access-requests/'+q.id+'/decision','POST',{request_digest:q.request_digest,decision:'allowed'},owner);assert(grant.next_turn_required);assert.equal((await waitJob(request.id)).state,'interrupted');await noContainer(s.id);
 await api('/v1/access-requests/'+q.id+'/decision','POST',{request_digest:q.request_digest,decision:'allowed'},owner,409);
 const read=await success(s,{tool:'bash',args:{command:`cat ${grant.target}/allowed.txt; if echo changed > ${grant.target}/allowed.txt; then echo WRITE_BREACH; else echo READ_ONLY; fi`},echo:true});assert(read.result.includes('READ_ONLY_GRANTED'));assert(!read.result.includes('WRITE_BREACH'));assert.equal(readFileSync(join(external,'allowed.txt'),'utf8'),'READ_ONLY_GRANTED\n');
 const revoking=await submit(s,{tool:'bash',args:{command:`while true; do cat ${grant.target}/allowed.txt >> grant-counter.txt; sleep 0.2; done`},final:'must stop'});
 await waitToolStart(revoking,'grant-counter.txt');await api('/v1/workspaces/'+w.id+'/grants/'+q.id+'/revoke','POST',{},owner);assert.equal((await waitJob(revoking.id)).state,'cancelled');await noContainer(s.id);
 const absent=await success(s,{tool:'bash',args:{command:`test ! -e ${grant.target}/allowed.txt && echo REVOKED`},echo:true});assert(absent.result.includes('REVOKED'));
 checks.push('typed owner path grant, stale digest/replay denial, read-only mount, stopped writers and revocation');
 const writeRequest=await submit(s,{tool:'ariel_request_path',args:{path:external,access:'write'},final:'MUST_STOP'}),wq=await question(writeRequest);const wg=await api('/v1/access-requests/'+wq.id+'/decision','POST',{request_digest:wq.request_digest,decision:'allowed'},owner);assert.equal((await waitJob(writeRequest.id)).state,'interrupted');
 await success(s,{tool:'write',args:{path:wg.target+'/written.txt',content:'WRITE_GRANTED\n'},final:'WRITE_OK'});assert.equal(readFileSync(join(external,'written.txt'),'utf8'),'WRITE_GRANTED\n');await api('/v1/workspaces/'+w.id+'/grants/'+wq.id+'/revoke','POST',{},owner);checks.push('explicit owner write grant applies only on the next turn');
 const asking=await submit(s,{tool:'ariel_ask_user',args:{method:'input',title:'Fixture clarification',prompt:'Enter an answer'},echo:true});const aq=await question(asking);assert.equal(aq.kind,'question');
 await api('/v1/questions/'+aq.id+'/answer','POST',{request_digest:aq.request_digest,value:'USER_ANSWER'});assert((await waitJob(asking.id)).result.includes('USER_ANSWER'));
 await api('/v1/questions/'+aq.id+'/answer','POST',{request_digest:aq.request_digest,value:'late'},client,409);checks.push('real Pi clarification: typed client answer and late-answer rejection');
 const crashing=await submit(s,{tool:'bash',args:{command:'while true; do echo tick >> crash-counter.txt; sleep 0.2; done'},final:'must stop'});await waitToolStart(crashing,'crash-counter.txt');
 const previousRequests=requests;daemon.kill();await new Promise(r=>daemon.once('exit',r));await start();
 const recovered=await api('/v1/jobs/'+crashing.id);assert.equal(recovered.state,'interrupted');assert.equal(recovered.error,'daemon_restart_unknown_outcome');await noContainer(s.id);await sleep(400);assert.equal(requests,previousRequests);assert.equal((await api('/v1/agents/pi/models')).models.length,1);
 checks.push('daemon crash recovery: interrupted outcome, no redispatch, owned process cleanup and persisted enablement');
 await stop();
 const evidence={at:new Date().toISOString(),kind:'deterministic-model-real-pi-docker',pi_version:'0.87.1',image:config.image_id,checks,model_requests:requests};writeFileSync('test-results/mvp-conformance.json',JSON.stringify(evidence,null,2));console.log(JSON.stringify(evidence,null,2));
}finally{if(daemon?.exitCode===null){try{await stop();}catch{daemon.kill();}}await new Promise(r=>fixture.close(r));}
async function waitToolStart(job,file){const sessions=(await api('/v1/sessions')).sessions,s=sessions.find(s=>s.id===job.session_id);const path=join(data,'workspaces',s.workspace_id,'tree',file);for(let i=0;i<100;i++){const j=await api('/v1/jobs/'+job.id);if(terminal(j.state))throw Error('Expected running tool: '+j.error);if(existsSync(path))return;await sleep(200);}throw Error('Tool startup timeout');}
