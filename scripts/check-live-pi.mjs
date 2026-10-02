// Explicit live smoke: uses the imported default Pi credentials; may consume provider quota.
import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {randomUUID,createHash} from 'node:crypto';
import assert from 'node:assert/strict';
const root=resolve(process.argv[2]||'.ariel'),config=JSON.parse(readFileSync(join(root,'config.json')));
const base='http://'+config.bind,owner=config.owner_token,client=config.clients[0].token;
async function api(path,method='GET',body,token=client,expected=200){
 const r=await fetch(base+path,{method,headers:{Authorization:'Bearer '+token,...(body?{'Content-Type':'application/json'}:{})},body:body?JSON.stringify(body):undefined});
 const value=await r.json();assert.equal(r.status,expected,JSON.stringify(value));return value;
}
const summary=JSON.parse(readFileSync(join(root,'profile/import-summary.json')));
const {catalog}=await api('/v1/admin/catalog','GET',null,owner);
const model=catalog.models.find(m=>m.provider_id===summary.defaultProvider&&m.model_id===summary.defaultModel);
assert(model?.configured,'Imported default model must be in the configured Pi catalog');
await api('/v1/admin/providers/'+encodeURIComponent(model.provider_id),'PUT',{enabled:true},owner);
await api('/v1/admin/models/'+model.selection_id,'PUT',{enabled:true},owner);
const w=await api('/v1/workspaces','POST',{mode:'folder'});
const s=await api('/v1/sessions','POST',{agent_id:'pi',provider_id:model.provider_id,model_id:model.model_id,workspace_id:w.id});
async function turn(input){
 const request={session_id:s.id,input,idempotency_key:randomUUID(),timeout_seconds:120};
 const initial=await api('/v1/jobs','POST',request);const duplicate=await api('/v1/jobs','POST',request);assert.equal(duplicate.id,initial.id);
 let job;const end=Date.now()+150000;
 while(Date.now()<end){job=await api('/v1/jobs/'+initial.id);if(['succeeded','failed','interrupted','cancelled'].includes(job.state))break;await new Promise(r=>setTimeout(r,1000));}
 console.log(JSON.stringify({job_id:initial.id,state:job.state,error:job.error}));
 assert.equal(job.state,'succeeded','Live Pi turn must reach an authoritative success');
 const r=await fetch(base+'/v1/jobs/'+job.id+'/events?after=0',{headers:{Authorization:'Bearer '+client}});const events=(await r.text()).split('\n').filter(l=>l.startsWith('data:')).map(l=>JSON.parse(l.slice(5)));
 assert.equal(events.filter(e=>e.kind==='terminal').length,1);assert(events.some(e=>e.kind==='tool_end'),'Pi must execute a tool');
 const cursor=events.at(-1).cursor;const tail=await fetch(base+'/v1/jobs/'+job.id+'/events?after='+cursor,{headers:{Authorization:'Bearer '+client}});assert.equal((await tail.text()).trim(),'');
 return {job,events};
}
const first=await turn('Create mvp-smoke.txt in the current working directory. Its entire content must be ARIEL_MVP_OK followed by a newline. Use your write tool. Reply briefly after it is written.');
const f=await api('/v1/workspaces/'+w.id+'/file?path=mvp-smoke.txt');assert.equal(f.text,'ARIEL_MVP_OK\n');
const second=await turn('Continue this session. Read the file you created in the previous turn with your read tool and reply with its exact marker. Do not modify the file.');
assert(second.job.result.includes('ARIEL_MVP_OK'));
await api('/v1/jobs/'+first.job.id,'GET',null,'invalid',401);
const evidence={at:new Date().toISOString(),kind:'live-provider',pi_version:'0.87.1',image:config.image_id,provider:model.provider_id,model:model.model_id,workspace:w.id,session:s.id,jobs:[first.job.id,second.job.id],file_sha256:createHash('sha256').update(f.text).digest('hex'),checks:['live Pi write','exact model selection','session continuation','idempotent retry','durable terminal SSE replay','authentication']};
mkdirSync('test-results',{recursive:true});writeFileSync('test-results/live-pi.json',JSON.stringify(evidence,null,2));console.log(JSON.stringify(evidence,null,2));
