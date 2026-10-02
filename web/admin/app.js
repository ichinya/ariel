const $=id=>document.getElementById(id);
let token=sessionStorage.getItem('ariel-owner')||'', catalog, clients=[], page=0;
const titles={models:'Провайдеры и модели',workspaces:'Рабочие пространства',questions:'Вопросы и разрешения',clients:'Тестовый клиент',jobs:'Задания'};
const states={ready:'Готова к запуску',disabled:'Выключена',auth_required:'Нужна авторизация',runtime_unavailable:'Pi недоступен'};
function node(tag,text,cls){const e=document.createElement(tag);if(text!==undefined)e.textContent=text;if(cls)e.className=cls;return e;}
function notice(message,error=false){$('notice').textContent=message;$('notice').className='notice'+(error?' error':'');}
async function api(path,method='GET',body){const r=await fetch(path,{method,headers:{Authorization:'Bearer '+token,...(body?{'Content-Type':'application/json'}:{})},body:body?JSON.stringify(body):undefined});const v=await r.json();if(!r.ok)throw Error(v.error?.code||'Ошибка запроса');return v;}
async function action(fn){try{await fn();}catch(e){notice(e.message,true);}}
function renderModels(){
 const p=$('provider').value;const provider=catalog.providers.find(v=>v.id===p);$('provider-enabled').checked=provider?.enabled||false;
 const all=catalog.catalog.models.filter(m=>m.provider_id===p);$('auth-status').textContent=all.some(m=>m.configured)?'Авторизация настроена; доступ к модели проверяется при выполнении задания':'Авторизация не настроена';
 const search=$('search').value.toLowerCase();const models=all.filter(m=>(m.name+' '+m.model_id).toLowerCase().includes(search));page=Math.min(page,Math.max(0,Math.ceil(models.length/40)-1));
 $('model-rows').replaceChildren();
 for(const m of models.slice(page*40,(page+1)*40)){
  const row=node('tr'),check=node('input');check.type='checkbox';check.checked=m.enabled;check.setAttribute('aria-label','Включить '+m.model_id);
  check.onchange=()=>action(async()=>{check.disabled=true;await api('/v1/admin/models/'+m.selection_id,'PUT',{enabled:check.checked});m.enabled=check.checked;await load(false);});
  const cell=node('td');cell.append(check);const name=node('td',m.name);name.append(node('small',m.model_id));row.append(cell,name,node('td',states[m.readiness]||m.readiness,m.ready?'pill good':''));$('model-rows').append(row);
 }
 $('model-count').textContent=all.filter(m=>m.enabled).length+' включено из '+all.length;
 $('page').textContent=models.length?'Страница '+(page+1)+' из '+Math.ceil(models.length/40):'Моделей нет';
 $('previous').disabled=page===0;$('next').disabled=(page+1)*40>=models.length;
}
async function load(reset=true){
 const previous=$('provider').value;catalog=await api('/v1/admin/catalog');
 $('provider').replaceChildren(...catalog.providers.map(p=>{const o=node('option',p.id);o.value=p.id;return o;}));
 $('provider').value=previous||catalog.import?.defaultProvider||catalog.providers[0]?.id;
 if(reset)page=0;renderModels();$('runtime').textContent=catalog.runtime_ready?'Pi готов':'Нужен runtime-build';$('runtime').className='pill '+(catalog.runtime_ready?'good':'warn');
 $('login').classList.add('hidden');$('content').classList.remove('hidden');
 clients=(await api('/v1/admin/clients')).clients;
 $('workspace-client').replaceChildren(...clients.map(c=>{const o=node('option',c.id);o.value=c.id;return o;}));
 $('client-list').replaceChildren();
 for(const c of clients){const card=node('div',undefined,'workspace'),input=node('input');input.type='password';input.readOnly=true;input.value=c.token;const copy=node('button','Скопировать ключ');copy.onclick=()=>action(async()=>{await navigator.clipboard.writeText(c.token);notice('Ключ клиента скопирован');});card.append(node('p',c.id),input,copy);$('client-list').append(card);}
 const health=await (await fetch('/health')).json();$('client-link').href=health.test_client_url;
 await refreshWorkspaces();await refreshQuestions();await refreshJobs();
}
async function refreshWorkspaces(){
 const workspaces=(await api('/v1/workspaces')).workspaces;$('workspace-list').replaceChildren();
 for(const w of workspaces){
  const card=node('div',undefined,'workspace');card.append(node('strong',w.mode+' · '+w.id.slice(0,8)),node('p','Клиент: '+w.client_id+' · '+w.container_path,'muted small'));
  const files=node('button','Показать файлы');files.onclick=()=>action(async()=>{const data=await api('/v1/workspaces/'+w.id+'/files');const list=node('div',undefined,'stack');for(const path of data.files){const b=node('button',path);b.onclick=()=>action(async()=>{const f=await api('/v1/workspaces/'+w.id+'/file?path='+encodeURIComponent(path));const pre=node('pre',f.text);list.replaceChildren(pre);});list.append(b);}card.append(list);});card.append(files);
  for(const g of w.grants||[]){const line=node('p',g.target+' · '+g.access+' · до '+new Date(g.expires_at*1000).toLocaleTimeString());const revoke=node('button','Отозвать доступ','danger');revoke.onclick=()=>action(async()=>{await api('/v1/workspaces/'+w.id+'/grants/'+g.id+'/revoke','POST',{});await refreshWorkspaces();});line.append(revoke);card.append(line);}
  $('workspace-list').append(card);
 }
 if(!workspaces.length)$('workspace-list').append(node('p','Рабочих пространств пока нет','muted'));
}
let questionsSignature='';
async function refreshQuestions(){
 if(!token)return;const questions=(await api('/v1/questions')).questions;const signature=JSON.stringify(questions);if(signature===questionsSignature)return;questionsSignature=signature;$('question-list').replaceChildren();
 for(const q of questions){
  const card=node('div',undefined,'card question');card.append(node('h2',q.kind==='path'?'Запрос доступа к файлам':q.title||'Вопрос Pi'));
  if(q.kind==='path'){
   card.append(node('pre',q.host_path+'\nКанонический путь: '+q.canonical_path),node('p',(q.access==='read'?'Только чтение':'Чтение и запись')+' · сессия '+q.session_id.slice(0,8)),node('p','Разрешение остановит текущую попытку. Путь будет доступен в этой сессии в течение часа; продолжение требует нового запроса.','muted'));
   for(const [label,decision] of [['Разрешить на час','allowed'],['Запретить','denied']]){const b=node('button',label,decision==='allowed'?'primary':'');b.onclick=()=>action(async()=>{const result=await api('/v1/access-requests/'+q.id+'/decision','POST',{request_digest:q.request_digest,decision});notice(result.target?'Доступ подготовлен: '+result.target+'. Отправьте следующий запрос в той же сессии.':'Доступ запрещён');await refreshQuestions();await refreshWorkspaces();});card.append(b);}
  }else{
   card.append(node('p',q.message||q.placeholder||''));let input;
   if(q.method==='select'){input=node('select');for(const value of q.options||[]){const o=node('option',value);o.value=value;input.append(o);}}
   else if(q.method!=='confirm'){input=node('textarea');card.append(input);}
   if(input&&q.method==='select')card.append(input);
   const send=node('button','Ответить','primary');send.onclick=()=>action(async()=>{await api('/v1/questions/'+q.id+'/answer','POST',{request_digest:q.request_digest,...(q.method==='confirm'?{confirmed:true}:{value:input.value})});await refreshQuestions();});card.append(send);
   if(q.method==='confirm'){const deny=node('button','Нет');deny.onclick=()=>action(async()=>{await api('/v1/questions/'+q.id+'/answer','POST',{request_digest:q.request_digest,confirmed:false});await refreshQuestions();});card.append(deny);}
  }
  $('question-list').append(card);
 }
}
async function refreshJobs(){
 const jobs=(await api('/v1/jobs')).jobs;$('job-list').replaceChildren();
 for(const j of jobs){const c=node('div',undefined,'workspace');c.append(node('strong',j.state+' · '+j.id.slice(0,8)),node('p',j.input),node('pre',j.result||j.error||'Выполняется…'));$('job-list').append(c);}
}
document.querySelectorAll('[data-tab]').forEach(b=>b.onclick=()=>{document.querySelectorAll('[data-tab]').forEach(n=>n.classList.toggle('active',n===b));document.querySelectorAll('[data-panel]').forEach(p=>p.classList.toggle('hidden',p.dataset.panel!==b.dataset.tab));$('page-title').textContent=titles[b.dataset.tab];if(token)action(async()=>{if(b.dataset.tab==='workspaces')await refreshWorkspaces();if(b.dataset.tab==='questions')await refreshQuestions();if(b.dataset.tab==='jobs')await refreshJobs();});});
$('login-form').onsubmit=e=>{e.preventDefault();token=$('owner-token').value;action(async()=>{await load();sessionStorage.setItem('ariel-owner',token);$('owner-token').value='';});};
$('provider').onchange=()=>{page=0;renderModels();};$('search').oninput=()=>{page=0;renderModels();};
$('previous').onclick=()=>{page--;renderModels();};$('next').onclick=()=>{page++;renderModels();};
$('save-provider').onclick=()=>action(async()=>{const body={enabled:$('provider-enabled').checked};if($('api-key').value)body.api_key=$('api-key').value;await api('/v1/admin/providers/'+encodeURIComponent($('provider').value),'PUT',body);$('api-key').value='';await load(false);notice('Провайдер сохранён');});
$('refresh').onclick=()=>action(async()=>{await api('/v1/admin/refresh','POST',{});await load(false);notice('Каталог обновлён');});
$('import').onclick=()=>action(async()=>{await api('/v1/admin/import-pi','POST',{});await load(false);notice('Локальная авторизация импортирована. Создайте новые сессии для обновлённых профилей.');});
$('workspace-mode').onchange=()=>{$('source-label').classList.toggle('hidden',$('workspace-mode').value!=='worktree');};
$('workspace-form').onsubmit=e=>{e.preventDefault();action(async()=>{await api('/v1/workspaces','POST',{mode:$('workspace-mode').value,client_id:$('workspace-client').value,...($('workspace-mode').value==='worktree'?{source_path:$('source').value}:{})});await refreshWorkspaces();notice('Рабочее пространство создано');});};
setInterval(()=>{if(token&&!$('content').classList.contains('hidden'))refreshQuestions().catch(()=>{});},2500);
if(token)action(()=>load());
