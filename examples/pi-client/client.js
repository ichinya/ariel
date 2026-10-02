const $=id=>document.getElementById(id),terminal=s=>['succeeded','failed','cancelled','interrupted'].includes(s);
let questionsSignature='';
let url='',token='',models=[],sessions=[],workspaces=[],jobId='',cursor=0,streamController,pending;
function node(tag,text,cls){const e=document.createElement(tag);if(text!==undefined)e.textContent=text;if(cls)e.className=cls;return e;}
function notice(text,error=false){$('notice').textContent=text;$('notice').className='notice'+(error?' error':'');}
async function api(path,method='GET',body){const r=await fetch(url+path,{method,headers:{Authorization:'Bearer '+token,...(body?{'Content-Type':'application/json'}:{})},body:body?JSON.stringify(body):undefined});const v=await r.json();if(!r.ok)throw Error(v.error?.code||'Ошибка запроса');return v;}
async function action(fn){try{await fn();}catch(e){notice(e.message,true);}}
function options(select,items,label,key){const selected=select.value;select.replaceChildren(...items.map(v=>{const o=node('option',label(v));o.value=key(v);return o;}));if(items.some(v=>key(v)===selected))select.value=selected;}
async function load(){
 const a=await api('/v1/agents');options($('agent'),a.agents,a=>a.id,a=>a.id);
 models=(await api('/v1/agents/pi/models')).models;options($('model'),models,m=>m.provider_id+' / '+m.model_id,m=>m.selection_id);
 if(!models.length)notice('Нет включённых доступных моделей. Настройте провайдера и модель в панели владельца.');
 workspaces=(await api('/v1/workspaces')).workspaces;options($('workspace'),workspaces,w=>w.mode+' · '+w.id.slice(0,8),w=>w.id);
 sessions=(await api('/v1/sessions')).sessions;options($('session'),sessions,s=>s.model_id+' · '+s.id.slice(0,8),s=>s.id);
 const jobs=(await api('/v1/jobs')).jobs;options($('job-select'),jobs,j=>j.state+' · '+j.input.slice(0,45),j=>j.id);
 $('new-session').disabled=!models.length||!workspaces.length;$('session').onchange();
 $('connection').textContent='Подключён';$('connection').className='pill good';$('connected').classList.remove('hidden');
}
function persist(){sessionStorage.setItem('ariel-client',JSON.stringify({url,token,jobId,cursor,pending}));}
function state(j){$('job-state').textContent=j.state;$('job-state').className='pill '+(j.state==='succeeded'?'good':j.state==='failed'?'bad':'');$('cancel').disabled=terminal(j.state);if(terminal(j.state)){if(j.result!==null)$('output').textContent=j.result||'';if(j.error)notice(j.error,j.state==='failed');}}
async function selectJob(id,replay=true){
 streamController?.abort();jobId=id;if(replay){cursor=0;$('output').textContent='';$('tools').textContent='';}
 persist();const j=await api('/v1/jobs/'+id);state(j);$('job-select').value=id;
 await stream(id);await loadFiles();
}
function event(record){
 cursor=record.cursor;persist();
 if(record.kind==='text')$('output').textContent+=record.data.delta;
 if(record.kind==='tool_start')$('tools').textContent+='\n▶ '+record.data.tool+' '+record.data.args+'\n';
 if(record.kind==='tool_end')$('tools').textContent+='✓ '+(record.data.tool||'инструмент')+' '+record.data.result+'\n';
 if(record.kind==='attention'){notice(record.data.kind==='path'?'Pi ожидает разрешение владельца на дополнительный путь':'Pi ожидает ответа');$('job-state').textContent='waiting_input';}
 if(record.kind==='terminal')state(record.data);
}
async function stream(id){
 const controller=new AbortController();streamController=controller;
 try{
  const response=await fetch(url+'/v1/jobs/'+id+'/events?after='+cursor,{headers:{Authorization:'Bearer '+token},signal:controller.signal});if(!response.ok)throw Error('Не удалось получить события');
  const reader=response.body.getReader(),decoder=new TextDecoder();let buffer='';
  while(true){const {value,done}=await reader.read();if(done)break;buffer+=decoder.decode(value,{stream:true}).replace(/\r\n/g,'\n');let end;while((end=buffer.indexOf('\n\n'))>=0){const frame=buffer.slice(0,end);buffer=buffer.slice(end+2);const data=frame.split('\n').filter(l=>l.startsWith('data:')).map(l=>l.slice(5).trimStart()).join('\n');if(data)event(JSON.parse(data));}}
  const j=await api('/v1/jobs/'+id);if(jobId===id){state(j);if(!terminal(j.state))setTimeout(()=>{if(jobId===id&&!controller.signal.aborted)stream(id).catch(e=>notice(e.message,true));},1500);}
 }catch(e){if(e.name!=='AbortError'){notice(e.message,true);setTimeout(()=>{if(jobId===id&&!controller.signal.aborted)stream(id).catch(e=>notice(e.message,true));},1500);}}
}
async function loadFiles(){
 if(!$('workspace').value)return;const wid=$('workspace').value;const data=await api('/v1/workspaces/'+wid+'/files');$('file-list').replaceChildren();
 for(const path of data.files){const b=node('button',path);b.onclick=()=>action(async()=>{const f=await api('/v1/workspaces/'+wid+'/file?path='+encodeURIComponent(path));$('file-preview').textContent=f.text;$('file-preview').classList.remove('hidden');});$('file-list').append(b);}
}
async function send(body){
 pending=body;persist();$('retry').classList.remove('hidden');
 const j=await api('/v1/jobs','POST',body);pending=null;persist();$('retry').classList.add('hidden');await load();selectJob(j.id).catch(e=>notice(e.message,true));
}
$('connect-form').onsubmit=e=>{e.preventDefault();action(async()=>{url=$('api-url').value.replace(/\/$/,'');token=$('client-token').value;await load();persist();if(jobId)selectJob(jobId,true).catch(e=>notice(e.message,true));});};
$('disconnect').onclick=()=>{streamController?.abort();token='';jobId='';cursor=0;sessionStorage.removeItem('ariel-client');$('client-token').value='';$('connected').classList.add('hidden');$('connection').textContent='Не подключён';};
$('refresh').onclick=()=>action(load);
$('new-folder').onclick=()=>action(async()=>{const w=await api('/v1/workspaces','POST',{mode:'folder'});await load();$('workspace').value=w.id;$('session').value='';$('submit').disabled=true;notice('Новая папка создана. Начните сессию для выбранной модели.');});
$('new-session').onclick=()=>action(async()=>{const m=models.find(m=>m.selection_id===$('model').value);if(!m)throw Error('Выберите модель');const s=await api('/v1/sessions','POST',{agent_id:'pi',provider_id:m.provider_id,model_id:m.model_id,workspace_id:$('workspace').value});await load();$('session').value=s.id;$('session').onchange();notice('Сессия создана');});
$('session').onchange=()=>{const s=sessions.find(s=>s.id===$('session').value);$('submit').disabled=true;if(s){$('workspace').value=s.workspace_id;const m=models.find(m=>m.provider_id===s.provider_id&&m.model_id===s.model_id);if(m){$('model').value=m.selection_id;$('submit').disabled=false;}}};
for(const id of ['model','workspace'])$(id).onchange=()=>{$('session').value='';$('submit').disabled=true;notice('Начните сессию для выбранной модели и пространства.');};
$('prompt-form').onsubmit=e=>{e.preventDefault();action(async()=>{if(!$('session').value)throw Error('Создайте сессию');await send({session_id:$('session').value,input:$('prompt').value,idempotency_key:crypto.randomUUID()});});};
$('retry').onclick=()=>action(()=>pending?send(pending):Promise.resolve());
$('cancel').onclick=()=>action(async()=>{if(jobId){const j=await api('/v1/jobs/'+jobId+'/cancel','POST',{});state(j);notice('Отмена запрошена; ожидается остановка Pi');}});
$('job-select').onchange=()=>action(()=>selectJob($('job-select').value));
$('files').onclick=()=>action(loadFiles);
setInterval(async()=>{if(!token||!jobId)return;try{const all=(await api('/v1/questions')).questions.filter(q=>q.job_id===jobId);const signature=JSON.stringify(all);if(signature!==questionsSignature){questionsSignature=signature;$('questions').replaceChildren();for(const q of all){const card=node('div',undefined,'notice');if(q.kind==='path'){card.textContent='Требуется разрешение владельца. Откройте панель Ариеля.';}else{card.append(node('p',q.title||q.message||'Вопрос Pi'));const input=node(q.method==='select'?'select':'input');if(q.method==='select')for(const option of q.options||[]){const o=node('option',option);o.value=option;input.append(o);}card.append(input);const yes=node('button','Ответить');yes.onclick=()=>action(async()=>{await api('/v1/questions/'+q.id+'/answer','POST',{request_digest:q.request_digest,...(q.method==='confirm'?{confirmed:true}:{value:input.value})});questionsSignature='';});card.append(yes);} $('questions').append(card);}}const j=await api('/v1/jobs/'+jobId);state(j);}catch{}},2500);
try{const previous=JSON.parse(sessionStorage.getItem('ariel-client')||'null');if(previous){({url,token,jobId,cursor,pending}=previous);$('api-url').value=url;$('client-token').value=token;if(pending){$('prompt').value=pending.input;$('retry').classList.remove('hidden');}action(async()=>{await load();if(jobId)selectJob(jobId,true).catch(e=>notice(e.message,true));});}}catch{}
