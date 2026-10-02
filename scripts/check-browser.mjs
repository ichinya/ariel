import {readFileSync,mkdirSync,writeFileSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
import assert from 'node:assert/strict';
import {randomUUID} from 'node:crypto';
const bin=process.env.AGENT_BROWSER_BIN;if(!bin)throw Error('Set AGENT_BROWSER_BIN to the agent-browser executable');
const config=JSON.parse(readFileSync(join(resolve(process.argv[2]||'.ariel'),'config.json')));
const checks=['owner panel authentication','provider/model controls','independent client authentication and catalogue','no browser errors'];
const suffix=randomUUID().slice(0,8);
function run(session,...args){const r=spawnSync(bin,['--session',session+'-'+suffix,...args],{encoding:'utf8',windowsHide:true,timeout:30000});assert.equal(r.status,0,'Browser command failed: '+args[0]);return r.stdout;}
mkdirSync('test-results',{recursive:true});
run('ariel-owner','open','http://'+config.bind);
run('ariel-owner','fill','#owner-token',config.owner_token);run('ariel-owner','click','#login-form button');
run('ariel-owner','wait','#provider');
console.log('Owner panel loaded.');
assert(run('ariel-owner','eval',"document.getElementById('content').classList.contains('hidden') ? 'hidden' : 'visible'").includes('visible'));
run('ariel-owner','screenshot',resolve('test-results/owner.png'));
assert.equal(run('ariel-owner','errors').trim(),'');
run('ariel-client','open','http://'+config.test_bind);
run('ariel-client','fill','#client-token',config.clients[0].token);run('ariel-client','click','#connect-form .primary');
run('ariel-client','wait','#connected:not(.hidden)');
console.log('Independent client connected.');
run('ariel-client','eval',"document.getElementById('client-token').value=''" );
const selections=run('ariel-client','eval',"JSON.stringify({agents:document.getElementById('agent').options.length,models:document.getElementById('model').options.length,sessions:document.getElementById('session').options.length})");
assert.equal(JSON.parse(JSON.parse(selections)).models,1);
let liveJob;
if(process.env.ARIEL_BROWSER_LIVE==='1'){
 const value=expression=>JSON.parse(run('ariel-client','eval',expression));
 const until=async predicate=>{for(let i=0;i<60;i++){if(await predicate())return;await new Promise(r=>setTimeout(r,500));}throw Error('Browser action timeout');};
 const api=async path=>{const r=await fetch('http://'+config.bind+path,{headers:{Authorization:'Bearer '+config.clients[0].token}});assert(r.ok);return r.json();};
 const previousWorkspace=value("document.getElementById('workspace').value");run('ariel-client','click','#new-folder');
 await until(()=>value("document.getElementById('workspace').value")!==previousWorkspace);
 const workspace=value("document.getElementById('workspace').value");assert.equal(value("document.getElementById('submit').disabled"),true);
 run('ariel-client','click','#new-session');await until(()=>value("document.getElementById('session').value")!=='');
 const session=value("document.getElementById('session').value"),prompt='Create browser-smoke.txt using your write tool. Its entire content must be ARIEL_BROWSER_OK followed by a newline. Reply briefly after writing it.';
 run('ariel-client','fill','#prompt',prompt);run('ariel-client','eval',"document.getElementById('submit').scrollIntoView({block:'center'})");run('ariel-client','snapshot','-i');run('ariel-client','click','#submit');
 await until(async()=>{liveJob=(await api('/v1/jobs')).jobs.find(j=>j.session_id===session&&j.input===prompt);return liveJob;});
 const end=Date.now()+150000;while(Date.now()<end){liveJob=await api('/v1/jobs/'+liveJob.id);if(['succeeded','failed','interrupted','cancelled'].includes(liveJob.state))break;await new Promise(r=>setTimeout(r,500));}
 assert.equal(liveJob.state,'succeeded',liveJob.error);assert.equal((await api('/v1/workspaces/'+workspace+'/file?path=browser-smoke.txt')).text,'ARIEL_BROWSER_OK\n');
 await until(()=>value("document.getElementById('job-state').textContent")==='succeeded');
 assert(value("document.getElementById('tools').textContent").includes('write'));
 checks.push('live UI creates folder and exact-model session, submits Pi file job, streams tool and final outcome');
}
run('ariel-client','screenshot',resolve('test-results/client.png'));
assert.equal(run('ariel-client','errors').trim(),'');
writeFileSync('test-results/browser.json',JSON.stringify({at:new Date().toISOString(),kind:liveJob?'live-provider-browser':'browser',pi_version:'0.87.1',image:config.image_id,checks,selections,live_job_id:liveJob?.id},null,2));
console.log('Owner and independent test client verified in Chromium; no browser errors.');
run('ariel-owner','close');run('ariel-client','close');
