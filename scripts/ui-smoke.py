import asyncio, json, os, shutil, tempfile, threading
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from playwright.async_api import async_playwright

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = Path(os.environ.get('KEEL_SMOKE_OUTPUT', Path(tempfile.gettempdir()) / 'keel-ui-smoke'))
OUTPUT.mkdir(parents=True, exist_ok=True)
PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j5uoAAAAASUVORK5CYII='
MOCK = r'''(() => {
if (window.top !== window.self) return;
localStorage.setItem('keel_welcome_done', '1');
const listeners = {}, sessions = [], jobs = [], projects = [];
let cfg = {selected_model_id:'test',external_base_url:null,external_model_name:null,image_endpoint:null,recent_workspaces:[],harness_mode:'keel', custom_model_paths:{}};
let runtime = {state:'stopped',active_model_id:null,base_url:'http://127.0.0.1:18180/v1'};
const presets = ['chat','general','code','websites','docs','sheets','study','slides'].map(id => ({id,title:({chat:'Chat',general:'Work',code:'Code'})[id] || id[0].toUpperCase()+id.slice(1),summary:'A useful local workspace',starters:['Help me get started'],tools:[]}));
let files = [], fileContent = {};
const emit = (event,payload) => (listeners[event] || []).forEach(fn => fn({payload}));
window.__test = {calls:[], emit};
window.__TAURI__ = {event:{listen:async (event,fn) => { (listeners[event] ||= []).push(fn); }},core:{invoke:async (cmd,args={}) => {
 window.__test.calls.push({cmd,args});
 let result;
 switch(cmd) {
 case 'get_bootstrap_state': result={version:'0.7.2',config:cfg, runtime,models:[{id:'test',name:'Test local model',installed:true,damaged:false,quantization:'Q4',bytes:1000000000,detail:'Fast local model',tier:'fast'},{id:'broken-9b',name:'Damaged 9B',installed:false,damaged:true,damage_reason:'This model file is incomplete or damaged.',quantization:'Q2_0',bytes:2556640128,detail:'Balanced model',tier:'balanced'}].filter(m=>m.id!=='broken-9b'||window.__test.includeDamaged),sessions:sessions.map(s=>({...s})),projects,presets,harnesses:[{id:'keel',name:'Keel Harness',available:true},{id:'keel-cli',name:'Keel Code (CLI)',available:false}],code_execution:window.__test.codeExecution || {can_run_commands:true,sandbox:'landlock',notice:null},mcp_servers:[],ocr_available:true,system_ram_gb:16};break;
 case 'refresh_mcp_servers': result=[];break;
 case 'toggle_mcp_server': result=[];break;
 case 'set_workspace_mcp_trust': result=[];break;
 case 'is_directory': result=false;break;
 case 'export_html_file': result='/test/export.html';break;
 case 'open_models_folder': result=null;break;
 case 'check_app_update': result=window.__test.fakeUpdate ? {current_version:'0.7.2',available:true,version:'0.8.0',notes:'In-app update test',release_url:'https://github.com/olivermikkola/keel-desktop/releases/tag/v0.8.0',can_install_in_place:true} : {current_version:'0.7.2',available:false,version:null,notes:null,release_url:'https://github.com/olivermikkola/keel-desktop/releases/latest',can_install_in_place:true};break;
 case 'install_app_update': result=null;break;
 case 'discover_models_cmd': result=[{id:'hf:prism-ml/Ternary-Bonsai-2.5-27B-GGUF',repo_id:'prism-ml/Ternary-Bonsai-2.5-27B-GGUF',name:'Ternary-Bonsai-2.5-27B',author:'prism-ml',likes:420,downloads:12000,trending_score:98.5,file_name:'Ternary-Bonsai-2.5-27B-Q2_0.gguf',resolve_url:'https://huggingface.co/prism-ml/Ternary-Bonsai-2.5-27B-GGUF/resolve/main/Ternary-Bonsai-2.5-27B-Q2_0.gguf',repo_url:'https://huggingface.co/prism-ml/Ternary-Bonsai-2.5-27B-GGUF',bytes:7670000000,quantization:'Q2_0',architecture:'qwen3',context_size:32768}];break;
 case 'scan_user_models_folder': result={folder:'/test/Models',added_ids:[],models:[]};break;
 case 'import_local_gguf': result=null;break;
 case 'import_documents_from_paths': result=(args.paths||[]).map(p=>({id:crypto.randomUUID(),name:String(p).split(/[\\/]/).pop()||'dropped.txt',kind:'txt',text:'Dropped document',truncated:false}));break;
 case 'import_document_from_bytes': result={id:crypto.randomUUID(),name:args.name || 'pasted.txt',kind:'txt',text:'Imported bytes',truncated:false};break;
 case 'create_session': result={id:crypto.randomUUID(),title:'New '+args.preset+' chat',app_preset:args.preset,workspace_path:args.workspacePath,project_id:args.projectId || null,pinned:false,messages:[],staged_edits:[],updated_at:Date.now()/1000};sessions.unshift(result);break;
 case 'load_session_cmd':result=sessions.find(s=>s.id===args.id);break;
 case 'list_workspace_files_cmd':result=files;break;
 case 'list_image_jobs':result=jobs;break;
 case 'list_video_jobs':result=window.__videoJobs || (window.__videoJobs=[]);break;
 case 'detect_video_tools':result={ffmpeg:'/usr/bin/ffmpeg'};break;
 case 'start_video_job': {
  window.__videoJobs = window.__videoJobs || [];
  const vjob={id:crypto.randomUUID(),request:args.request,workspace_path:args.workspacePath,status:'succeeded',artifact_path:args.request.path,manifest_path:'videos/keel-short.json',storyboard_path:'videos/keel-short.storyboard.md',plan:{path:args.request.path,title:args.request.title||null,voice:'local',aspect_ratio:'9:16',width:1080,height:1920,clips:[{image_path:'assets/keel-test.png',narration:'Scene one.',overlay:'HOOK',motion:'slow_zoom',duration:2.5,start:0}],total_seconds:2.5},used_tts:false,error:null,created_at:Date.now()/1000};
  window.__videoJobs.unshift(vjob); result=vjob;
  setTimeout(()=>emit('keel://video-job',vjob),100); break;
 }
 case 'read_workspace_video':result='data:video/mp4;base64,AAAAIGZ0eXBpc29tAAACAGlzb21pc28y';break;
 case 'list_photo_slides':result=window.__photoSlides || (window.__photoSlides=[]);break;
 case 'build_photo_slides': {
  window.__photoSlides = window.__photoSlides || [];
  const deck={id:crypto.randomUUID(),title:(args.request?.slides?.[0]?.headline)||'Photo Slides',description:args.request?.description||'',html_path:args.request?.path||'photo-slides/demo.html',manifest_path:'photo-slides/demo.json',workspace_path:args.workspacePath,slide_count:(args.request?.slides||[]).length,slides:args.request?.slides||[],created_at:Date.now()/1000};
  window.__photoSlides.unshift(deck);
  fileContent[deck.html_path]='<!doctype html><html><body><h1>'+deck.title+'</h1></body></html>';
  result=deck; break;
 }
 case 'read_photo_slides_html':result=fileContent[args.relPath] || '<!doctype html><html><body><h1>Photo Slides</h1></body></html>';break;
 case 'generate_photo_slides_text':result=[{kind:'cover',headline:'Weekend in Porvoo',body:'Four stops worth the trip.'},{kind:'photo',headline:'Old town bridge',body:'Morning coffee by the river.'},{kind:'closing',headline:'Save for later',body:'Easy day trip from Helsinki.'}];break;
 case 'get_runtime_status':result=runtime;break;
 case 'pick_documents':result=[{id:crypto.randomUUID(),name:'brief.txt',kind:'txt',text:'Project Harbor budget is 4800 euros.',truncated:false},{id:crypto.randomUUID(),name:'receipt.png',kind:'png',text:'Total due: 480 EUR',truncated:false,image_data_url:''' + json.dumps(PNG) + r'''}];break;
 case 'save_personal_context':cfg.personal_context=args.text;result=null;break;
 case 'save_project':{const old=projects.findIndex(p=>p.id===args.project.id);if(old>=0)projects[old]=args.project;else projects.push(args.project);result=projects;break;}
 case 'get_project':result=projects.find(p=>p.id===args.id);break;
 case 'search_threads':result=sessions.filter(s=>(!args.projectId||s.project_id===args.projectId)&&(s.title+' '+s.messages.map(m=>m.content).join(' ')).toLowerCase().includes(args.query.toLowerCase()));break;
 case 'update_thread':{const thread=sessions.find(s=>s.id===args.id);Object.assign(thread,{title:args.title,pinned:args.pinned,project_id:args.projectId});result=thread;break;}
 case 'fork_thread':{const previous=sessions.find(s=>s.id===args.id);const index=previous.messages.findIndex(m=>m.id===args.beforeMessageId);result={...structuredClone(previous),id:crypto.randomUUID(),messages:previous.messages.slice(0,index),staged_edits:[]};sessions.unshift(result);break;}
 case 'list_trashed_threads':result=[];break;
 case 'empty_trash':result=null;break;
 case 'list_library':result=files.map(path=>({id:path,name:path,path,kind:path.split('.').pop(),workspace_path:'/test/project',project_id:projects[0]?.id || null,updated_at:Date.now()/1000}));break;
 case 'get_setup_plan':result={memory_bytes:17179869184,runtime_path:null,runtime_asset:{bytes:20000000},recommended_model:{id:'test',name:'Recommended model',detail:'A balanced local assistant',bytes:1000000000}};break;
 case 'setup_local_ai':runtime={...runtime,state:'running',active_model_id:'test'};result=runtime;break;
 case 'pick_workspace_folder':result=window.__test.cancelPicker ? null : '/test/project';break;
 case 'prepare_sample_workspace':result='/test/Sample Work';break;
 case 'set_workspace_folder': {
 const s=sessions.find(s=>s.id===args.sessionId); if(s) s.workspace_path=args.path;
 cfg.recent_workspaces=[args.path];result=args.path;break;}
 case 'start_model':runtime={...runtime,state:'running',active_model_id:args.modelId || 'test'};cfg.external_base_url=null;cfg.external_model_name=null;result=runtime;break;
 case 'auto_start_model':runtime={...runtime,state:'running',active_model_id:'test'};result=runtime;break;
 case 'stop_runtime':runtime={...runtime,state:'stopped',active_model_id:null};cfg.external_base_url=null;cfg.external_model_name=null;result=runtime;break;
 case 'clear_external_endpoint':runtime={...runtime,state:'stopped',active_model_id:null};cfg.external_base_url=null;cfg.external_model_name=null;result=runtime;break;
 case 'download_catalog_model':window.__test.includeDamaged=false;result=[{id:'test',name:'Test local model',installed:true,damaged:false,quantization:'Q4',bytes:1000000000,detail:'Fast local model',tier:'fast'}];break;
 case 'delete_model':window.__test.includeDamaged=false;result=[{id:'test',name:'Test local model',installed:true,damaged:false,quantization:'Q4',bytes:1000000000,detail:'Fast local model',tier:'fast'}];break;
 case 'pick_model_file':result=null;break;
 case 'set_external_endpoint':if(!String(args.baseUrl||'').match(/127\.0\.0\.1|localhost|\[::1\]/i) && !args.confirmRemote) throw new Error('This server is not on this computer.');cfg.external_base_url=args.baseUrl;cfg.external_model_name=args.modelName || null;runtime={state:'external',active_model_id:null,base_url:args.baseUrl};result=runtime;break;
 case 'connect_image_server':cfg.image_endpoint=args.endpoint;result=['Local checkpoint'];break;
 case 'disconnect_image_server':cfg.image_endpoint=null;result=null;break;
 case 'read_workspace_png':result=''' + json.dumps(PNG) + r''';break;
 case 'read_workspace_image':result=''' + json.dumps(PNG) + r''';break;
 case 'read_workspace_file_raw':result=fileContent[args.relPath] || '# Local document';break;
 case 'send_message': {
 const s=sessions.find(s=>s.id===args.input.session_id);
 s.messages.push({id:'u'+Date.now(),role:'user',content:args.input.prompt,attachments:args.input.attachments || []});
 let tool_calls=[];
 if(s.app_preset!=='chat') {
 const edit={id:crypto.randomUUID(),path:'brief.md',summary:'Create brief.md',diff:'--- brief.md\n+++ brief.md\n+# Brief',added_lines:1,removed_lines:0,status:'pending'};
 s.staged_edits.push(edit);
 tool_calls=[{name:'create_file',arguments:{path:'brief.md'},result:'Staged for review',edit_id:edit.id,is_error:false}];
 emit('keel://agent-event',{type:'staged_edit_added',edit});
 }
 s.messages.push({id:'a'+Date.now(),role:'assistant',content:'Ready to help.',tool_calls});s.title=args.input.prompt;
 result=s;break;}
 case 'apply_staged_edit_cmd': {const s=sessions.find(s=>s.id===args.sessionId);s.staged_edits.find(e=>e.id===args.editId).status='applied'; files.push('brief.md');fileContent['brief.md']='# Brief';result=s;break;}
 case 'create_code_worktree': {const s=sessions.find(s=>s.id===args.sessionId);s.code_checkout={original:s.workspace_path,isolated:'/test/worktrees/'+s.id,base:'deadbeef12345678'};s.applied_code_patch=null;s.last_code_test=null;result=s;break;}
 case 'review_code_worktree': {result={paths:['calc.py','test_calc.py'],diff:'--- a/calc.py\n+++ b/calc.py\n@@ -1 +1 @@\n-return a - b\n+return a + b\n'};break;}
 case 'run_code_test': {const s=sessions.find(s=>s.id===args.sessionId);const pass=!args.command.includes('fail');s.last_code_test={command:args.command,exit_code:pass?0:1,output:pass?'2 passed':'1 failed',passed:pass};result=s;break;}
 case 'apply_code_worktree': {const s=sessions.find(s=>s.id===args.sessionId);s.applied_code_patch=args.patch;result=s;break;}
 case 'undo_code_worktree': {const s=sessions.find(s=>s.id===args.sessionId);s.applied_code_patch=null;result=s;break;}
 case 'remove_code_worktree': {const s=sessions.find(s=>s.id===args.sessionId);s.code_checkout=null;s.applied_code_patch=null;s.last_code_test=null;result=s;break;}
 case 'inspect_publish_status': {result={repo_root:'/test/project',branch:'main',head:'abc1234',dirty_files:['brief.md'],remote_name:'origin',remote_url:'https://github.com/example/repo.git',gh_available:true,gh_authenticated:true};break;}
 case 'publish_code_changes': {result={branch:args.plan.branch_name,commit_sha:'def5678',pushed:args.plan.push_remote,pr_url:args.plan.open_pr?'https://github.com/example/repo/pull/99':null,next_command:null};break;}
 case 'start_image_job': {
 const job={id:crypto.randomUUID(),request:args.request,workspace_path:args.workspacePath,status:'running',artifact_path:null,created_at:Date.now()/1000};jobs.unshift(job);result=job;
 setTimeout(()=>{job.status='succeeded';job.artifact_path='assets/keel-test.png';files.push(job.artifact_path);emit('keel://image-job',job);},250);break;}
 case 'list_audio_packs': result=window.__audioPacks || (window.__audioPacks=[
  {id:'stt-demo-fixture',capability:'stt',display_name:'Speech to text · Demo fixture (checksummed)',min_vram_mib:0,runner_kind:'audio_stt',download_url:'https://example.test/demo-stt.weights',sha256:'b3bbdb816ceaf3dccff5669d4e6fe361cdd92ccac7fc027e1015e204262d883e',file_name:'demo-stt.weights',bytes:43,installed:false,runner_path:null},
  {id:'stt-whisper-tiny-q5',capability:'stt',display_name:'Speech to text · Whisper tiny q5_1',min_vram_mib:512,runner_kind:'audio_stt',download_url:'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin',sha256:'818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7',file_name:'ggml-tiny-q5_1.bin',bytes:32152673,installed:false,runner_path:null},
  {id:'stt-whisper-base',capability:'stt',display_name:'Speech to text · Whisper base (unpinned)',min_vram_mib:1024,runner_kind:'audio_stt',download_url:'keel-pack:unpinned/stt-whisper-base',sha256:null,file_name:null,bytes:0,installed:false,runner_path:null},
  {id:'tts-local-en',capability:'tts',display_name:'Text to speech · English (stub)',min_vram_mib:512,runner_kind:'audio_tts',download_url:null,sha256:null,file_name:null,bytes:0,installed:false,runner_path:null}
 ]);break;
 case 'register_audio_pack_runner': {
  const packs=window.__audioPacks || [];
  const pack=packs.find(p=>p.id===args.packId); if(!pack) throw new Error('Unknown audio pack');
  pack.installed=true; pack.runner_path=args.command; result=structuredClone(pack); break;
 }
 case 'register_whisper_cpp_runner': {
  const packs=window.__audioPacks || [];
  const pack=packs.find(p=>p.id===args.packId); if(!pack) throw new Error('Unknown audio pack');
  if(!args.model && !pack.installed) throw new Error('Download this pack’s model or provide a local model file.');
  pack.installed=true; pack.runner_path=args.command; result=structuredClone(pack); break;
 }
 case 'download_audio_pack': {
  const packs=window.__audioPacks || [];
  const pack=packs.find(p=>p.id===args.packId); if(!pack) throw new Error('Unknown audio pack');
  if(!pack.sha256 || !pack.download_url || String(pack.download_url).startsWith('keel-pack:unpinned/')) throw new Error("Pack download is not pinned yet (need http(s) URL + sha256 + file_name).");
  pack.installed=true; result=structuredClone(pack); break;
 }
 case 'remove_audio_pack': {
  const packs=window.__audioPacks || [];
  const pack=packs.find(p=>p.id===args.packId); if(!pack) throw new Error('Unknown audio pack');
  pack.installed=false; pack.runner_path=null; result=null; break;
 }
 case 'list_audio_artifacts': result=window.__audioArtifacts || (window.__audioArtifacts=[]); break;
 case 'clear_audio_artifacts': window.__audioArtifacts=[]; result=null; break;
 case 'prepare_audio_pack_vram': result={status:'already_free',reason:'smoke',capability_id:'stt-whisper-base',usable_free_mib:0,runtime:{state:'stopped'}};break;
 case 'prepare_pack_vram': result={status:'already_free',reason:'smoke',capability_id:'demo.capability',usable_free_mib:0,runtime:{state:'stopped'}};break;
 case 'pick_audio_file': result=window.__test?.cancelAudioPicker ? null : '/test/clip.wav'; break;
 case 'register_piper_runner': {
  const packs=window.__audioPacks || [];
  const pack=packs.find(p=>p.id===args.packId); if(!pack) throw new Error('Unknown audio pack');
  if(!args.model) throw new Error('Provide a Piper ONNX model path.');
  pack.installed=true; pack.runner_path=args.command; result=structuredClone(pack); break;
 }
 case 'run_audio_stt': {
  const packs=window.__audioPacks || [];
  if(!packs.some(p=>p.capability==='stt'&&p.installed&&p.runner_path)) throw new Error('No local runner is registered for speech-to-text.');
  const text='Hello from smoke STT.';
  window.__audioArtifacts = window.__audioArtifacts || [];
  window.__audioArtifacts.unshift({kind:'stt_transcript',path:'/test/data/audio-artifacts/smoke-stt.txt',created_at:Date.now()/1000,text_preview:text});
  result=text; break;
 }
 case 'run_audio_tts': {
  const packs=window.__audioPacks || [];
  if(!packs.some(p=>p.capability==='tts'&&p.installed&&p.runner_path)) throw new Error('No local runner is registered for text-to-speech.');
  const out='/test/data/audio-try/try-tts.wav';
  window.__audioArtifacts = window.__audioArtifacts || [];
  window.__audioArtifacts.unshift({kind:'tts_output',path:out,created_at:Date.now()/1000,text_preview:args.text||''});
  result=out; break;
 }
 default: throw new Error('Unexpected command '+cmd);
 }
 return structuredClone(result);
}}};
})();'''


FIRST_RUN_LAYER = r'''(() => {
if (window.top !== window.self) return;
const t = window.__test, core = window.__TAURI__.core, base = core.invoke;
const big = {id:'bonsai-9b',name:'Bonsai 2 · 9B',detail:'Recommended everyday agent model.',bytes:2556640128};
const fast = {id:'bonsai-4b',name:'Bonsai 2 · 4B',detail:'Fast local starter model.',bytes:1212103520};
core.invoke = async (cmd,args={}) => {
 if (cmd==='get_bootstrap_state') { const r=await base(cmd,args); if(t.firstRun && !t.setupDone) r.models.forEach(m=>m.installed=false); return r; }
 if (cmd==='get_setup_plan') { t.calls.push({cmd,args}); return {memory_bytes:17179869184,platform:'test',runtime_path:null,runtime_asset:{bytes:20000000},recommended_model:big,fast_model:fast,context_size:8192}; }
 if (cmd==='pause_local_setup') { t.calls.push({cmd,args}); t.failSetup('Setup download paused'); return true; }
 if (cmd==='setup_local_ai') {
  t.calls.push({cmd,args}); t.emit('keel://setup-stage','runner');
  return new Promise((resolve,reject) => { t.finishSetup=async()=>{ t.setupDone=true; resolve(await base('setup_local_ai',args)); }; t.failSetup=(message)=>reject(message); });
 }
 if ((cmd==='start_model' || cmd==='auto_start_model') && t.holdStart) {
  t.calls.push({cmd,args});
  return new Promise((resolve) => { t.releaseStart=async()=>{ resolve(await base(cmd,args)); }; });
 }
 return base(cmd,args);
};
})();'''

def mock_for(first_run=False, hold_start=False):
    script = MOCK
    if first_run:
        script = script.replace("localStorage.setItem('keel_welcome_done', '1');", "")
    return script + FIRST_RUN_LAYER + f"if(window.__test){{window.__test.firstRun={'true' if first_run else 'false'};window.__test.holdStart={'true' if hold_start else 'false'};}}"

async def new_page(browser, port, **options):
    page = await browser.new_page(viewport={'width':1440,'height':1000})
    page.errors=[]
    page.on('pageerror', lambda e:page.errors.append(str(e)))
    await page.add_init_script(mock_for(**options))
    page.set_default_timeout(10000)
    await page.goto(f'http://127.0.0.1:{port}')
    return page

async def first_run(browser, port):
    """A brand-new person: welcome, one-click setup, live progress, a question asked while waiting, and a failed download."""
    page = await new_page(browser, port, first_run=True)
    await page.locator('#welcome-start').wait_for()
    overlay = await page.locator('#welcome-overlay').inner_text()
    assert 'Your own private AI.' in overlay, overlay
    for jargon in ('llama.cpp','MCP','sandbox','Metal','CUDA','stdio'):
        assert jargon not in overlay, jargon
    await page.screenshot(path=str(OUTPUT / 'first-run-welcome.png'))
    # The lighter download is one click away and is what gets installed when chosen.
    await page.locator('#welcome-size').click()
    await page.get_by_text('Use the smarter one instead', exact=False).wait_for()
    await page.locator('#welcome-start').click()
    await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='setup_local_ai'&&c.args.modelId==='bonsai-4b')")
    await page.locator('#welcome-bar').wait_for()
    # Live numbers: percent, speed and time left.
    await page.evaluate("window.__test.emit('keel://download-progress',{model_id:'runtime',downloaded_bytes:20000000,total_bytes:20000000,done:true})")
    await page.evaluate("window.__test.emit('keel://setup-stage','model')")
    await page.evaluate("window.__test.emit('keel://download-progress',{model_id:'bonsai-4b',downloaded_bytes:100000000,total_bytes:1212103520,done:false})")
    await page.wait_for_timeout(1300)
    await page.evaluate("window.__test.emit('keel://download-progress',{model_id:'bonsai-4b',downloaded_bytes:400000000,total_bytes:1212103520,done:false})")
    await page.wait_for_function("document.querySelector('#welcome-meter').textContent.includes('/s') && document.querySelector('#welcome-meter').textContent.includes('left')")
    meter = await page.locator('#welcome-meter').inner_text()
    pct = await page.locator('#welcome-pct').inner_text()
    assert pct.endswith('%') and 0 < int(pct[:-1]) < 100, pct
    assert 'of' in meter, meter
    await page.screenshot(path=str(OUTPUT / 'first-run-downloading.png'))
    # Boredom escape hatch: ask a question now; it is sent the moment the AI is ready.
    await page.locator('[data-chip="0"]').click()
    await page.locator('#welcome-overlay').wait_for(state='hidden')
    await page.get_by_text('Your message sends as soon as your AI is ready', exact=True).wait_for()
    assert await page.locator('#prompt-input').input_value() != ''
    assert not await page.evaluate("window.__test.calls.some(c=>c.cmd==='send_message')")
    await page.locator('.sidebar-runtime-state').get_by_text('Setting up', exact=False).wait_for()
    await page.screenshot(path=str(OUTPUT / 'first-run-waiting.png'))
    await page.evaluate("window.__test.finishSetup()")
    await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='send_message')")
    await page.get_by_text('Ready to help.', exact=True).wait_for()
    assert await page.evaluate("localStorage.getItem('keel_welcome_done')") == '1'
    assert not page.errors, page.errors
    await page.close()

    # A failed download explains itself in plain words and can be retried.
    page = await new_page(browser, port, first_run=True)
    await page.locator('#welcome-start').click()
    await page.locator('#welcome-bar').wait_for()
    await page.evaluate("window.__test.failSetup('error sending request for url (https://huggingface.co/x)')")
    await page.locator('#welcome-retry').wait_for()
    text = await page.locator('#welcome-overlay').inner_text()
    assert "couldn't reach the download server" in text and 'huggingface' not in text.split('Technical details')[0], text
    await page.screenshot(path=str(OUTPUT / 'first-run-error.png'))
    await page.locator('#welcome-retry').click()
    await page.wait_for_function("window.__test.calls.filter(c=>c.cmd==='setup_local_ai').length===2")
    await page.evaluate("window.__test.finishSetup()")
    await page.locator('#welcome-overlay').wait_for(state='hidden')
    assert not page.errors, page.errors
    await page.close()

    # Skipping is fine: the empty chat keeps a one-click way back, and the app never nags with the welcome again.
    page = await new_page(browser, port, first_run=True)
    await page.locator('#welcome-skip').click()
    await page.locator('#welcome-overlay').wait_for(state='hidden')
    await page.locator('#chat-setup-btn').get_by_text('Set up my AI', exact=True).wait_for()
    await page.reload()
    await page.get_by_role('heading', name='What can I help with?', exact=True).wait_for()
    assert await page.locator('#welcome-overlay').is_hidden()
    assert not page.errors, page.errors
    await page.close()

async def second_launch(browser, port):
    """The model is already installed: opening the app starts it, and a message typed early is not lost."""
    page = await new_page(browser, port, hold_start=True)
    await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='auto_start_model')")
    await page.locator('#model-pill-label').get_by_text('Waking up your AI', exact=False).wait_for()
    await page.locator('#prompt-input').fill('Hello early Keel')
    await page.locator('#send-btn').click()
    await page.get_by_text('Your message sends as soon as your AI is ready', exact=True).wait_for()
    assert not await page.evaluate("window.__test.calls.some(c=>c.cmd==='send_message')")
    await page.screenshot(path=str(OUTPUT / 'second-launch-waking.png'))
    await page.evaluate("window.__test.releaseStart()")
    await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='send_message'&&c.args.input.prompt==='Hello early Keel')")
    await page.get_by_text('Ready to help.', exact=True).wait_for()
    assert not page.errors, page.errors
    await page.close()

async def main(port):
    async with async_playwright() as p:
        browser = await p.chromium.launch(executable_path=os.environ.get('CHROMIUM_BIN') or shutil.which('chromium'), args=['--no-sandbox'])
        page = await browser.new_page(viewport={'width':1440,'height':1000})
        errors=[]
        page.on('pageerror', lambda e:errors.append(str(e)))
        await page.add_init_script(mock_for())
        page.set_default_timeout(10000)
        await page.goto(f'http://127.0.0.1:{port}')
        await page.get_by_role('heading', name='What can I help with?', exact=True).wait_for()
        assert await page.locator('#workspace-pill-btn').is_hidden()
        # The installed model starts by itself on launch; nobody has to find a button.
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='auto_start_model')")
        await page.locator('#model-pill-label').get_by_text('Test local model', exact=False).wait_for()
        # Quick model switcher opens right in place from the top pill.
        await page.locator('#model-pill-btn').click()
        await page.locator('#model-switcher-menu').get_by_text('ON THIS COMPUTER', exact=False).wait_for()
        await page.keyboard.press('Escape')
        await page.locator('#model-switcher-menu').wait_for(state='hidden')
        # Settings hides developer options (llama-server / Code engine) by default and supports Paper light theme.
        await page.locator('#open-customize-btn').click()
        assert await page.locator('#cust-llama').is_hidden()
        await page.locator('[data-tone-pick="paper"]').click()
        assert await page.evaluate("document.documentElement.dataset.tone === 'paper'")
        await page.locator('[data-tone-pick="ink"]').click()
        assert await page.evaluate("document.documentElement.dataset.tone === 'ink'")
        # Downloaded weights need a runner. Registering one enables the native contract.
        whisper = page.locator('[data-audio-pack="stt-whisper-tiny-q5"]')
        await whisper.locator('[data-audio-download]').click()
        await whisper.locator('[data-audio-remove]').wait_for()
        await page.locator('#audio-try-stt').click()
        await page.locator('#audio-try-status').get_by_text('No local runner is registered', exact=False).wait_for()
        prompts = iter(("whisper-cli", ""))
        async def answer_whisper_prompt(dialog):
            await dialog.accept(next(prompts))
        page.on('dialog', answer_whisper_prompt)
        await whisper.locator('[data-audio-whisper]').click()
        await whisper.get_by_text('Runner configured', exact=False).wait_for()
        page.remove_listener('dialog', answer_whisper_prompt)
        tts = page.locator('[data-audio-pack="tts-local-en"]')
        page.once('dialog', lambda dialog: dialog.accept('fixture-tts'))
        await tts.locator('[data-audio-runner]').click()
        await tts.get_by_text('Runner configured', exact=False).wait_for()
        await page.locator('#audio-try-stt').click()
        await page.locator('#audio-try-status').get_by_text('STT ok:', exact=False).wait_for()
        await page.locator('#audio-try-tts').click()
        await page.locator('#audio-try-status').get_by_text('TTS ok', exact=False).wait_for()
        await page.screenshot(path=str(OUTPUT / 'audio-runners.png'))
        await page.locator('#audio-artifacts-list').get_by_text('smoke-stt.txt', exact=False).wait_for()
        # App updates check in Settings surfaces the 1-click Update & restart banner.
        await page.evaluate("window.__test.fakeUpdate = true")
        await page.locator('#check-app-update-btn').click()
        await page.locator('#install-app-update-btn').get_by_text('Update & restart', exact=True).wait_for()
        await page.locator('#modal-close-btn').click()
        await page.locator('#update-banner').get_by_text('Keel v0.8.0 available', exact=True).wait_for()
        await page.locator('#update-banner-dismiss').click()
        await page.locator('#update-banner').wait_for(state='hidden')
        await page.locator('#prompt-input').fill('Hello Keel')
        await page.locator('#send-btn').click()
        await page.get_by_text('Ready to help.',exact=True).wait_for()
        await page.locator('#context-meter-pill').wait_for(state='visible')
        assert await page.locator('#viewer-pane').is_hidden()
        await page.screenshot(path=str(OUTPUT / 'chat.png'))
        # Attach a source, inspect it, then save explicit cross-conversation preferences.
        await page.locator('#attach-docs-btn').click()
        await page.locator('[data-pending-source]').first.wait_for()
        await page.locator('#prompt-input').fill('Summarize the attached brief')
        await page.locator('#send-btn').click()
        await page.locator('[data-message-source]').first.wait_for()
        await page.locator('[data-message-source]').first.click()
        await page.locator('.document-text').get_by_text('Project Harbor budget is 4800 euros.',exact=True).wait_for()
        await page.locator('#modal-close-btn').click()
        await page.locator('[data-message-source]').nth(1).click()
        await page.locator('.modal img.generated-image').wait_for()
        await page.locator('.document-text').get_by_text('Total due: 480 EUR',exact=True).wait_for()
        await page.locator('#modal-close-btn').click()
        await page.locator('#memory-btn').click()
        await page.locator('#personal-context').fill('I am vegetarian and prefer concise answers.')
        await page.locator('#memory-save').click()
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='save_personal_context'&&c.args.text.includes('vegetarian'))")
        # Project instructions and documents have a persistent, editable home.
        await page.locator('#new-project-btn').click()
        await page.locator('#project-name').fill('Harbor')
        await page.locator('#project-instructions').fill('Write for local cafes.')
        await page.locator('#project-add-docs').click()
        await page.locator('#project-save').click()
        await page.locator('[data-project]').filter(has_text='Harbor').wait_for()
        await page.get_by_role('heading', name='Chat', exact=True).wait_for()
        # Renaming/pinning keeps a thread manageable as history grows.
        await page.locator('#thread-settings-btn').click()
        await page.locator('#thread-title').fill('Harbor planning')
        await page.locator('#thread-pinned').check()
        await page.locator('#thread-save').click()
        await page.locator('#thread-search').fill('Harbor')
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='search_threads'&&c.args.query==='Harbor')")
        await page.locator('#thread-search').fill('')

        await page.locator('[data-view=work]').click()
        await page.get_by_role('heading',name='Work',exact=True).wait_for()
        await page.locator('#empty-open-folder').wait_for()
        await page.screenshot(path=str(OUTPUT / 'work-folder-first.png'))
        await page.evaluate("window.__test.cancelPicker = true")
        await page.locator('#prompt-input').fill('Make a project brief')
        await page.locator('#send-btn').click()
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='pick_workspace_folder')")
        assert await page.locator('#prompt-input').input_value() == 'Make a project brief'
        assert await page.evaluate("!window.__test.calls.some(c=>c.cmd==='send_message' && c.args.input.prompt==='Make a project brief')")
        await page.evaluate("window.__test.cancelPicker = false")
        await page.locator('#send-btn').click()
        await page.locator('[data-edit-approve]').wait_for()
        assert await page.evaluate("window.__test.calls.filter(c=>c.cmd==='send_message' && c.args.input.prompt==='Make a project brief' && c.args.input.workspace_path==='/test/project').length === 1")
        await page.screenshot(path=str(OUTPUT / 'work.png'))
        await page.locator('[data-edit-approve]').click()
        await page.get_by_text('Approved · written to workspace',exact=True).wait_for(state='attached')
        await page.get_by_role('heading',name='Brief',exact=True).wait_for()
        await page.locator('[data-view=code]').click()
        await page.get_by_role('heading',name='Code',exact=True).wait_for()
        assert await page.locator('#harness-pill-btn').is_visible()
        assert await page.locator('#viewer-pane').is_visible()
        await page.locator('#prompt-input').fill('Inspect this repository')
        await page.locator('#send-btn').click()
        await page.get_by_text('Ready to help.',exact=True).wait_for()
        await page.locator('[data-viewer-tab=diff]').click()
        await page.locator('#code-worktree-create').click()
        await page.locator('#code-test-cmd').fill('pytest -q --fail')
        await page.locator('#code-test-run').click()
        await page.locator('.worktree-test-result.failed').get_by_text('exit 1', exact=False).wait_for()
        await page.locator('#code-test-cmd').fill('pytest -q')
        await page.locator('#code-test-run').click()
        await page.locator('.worktree-test-result.passed').get_by_text('exit 0', exact=False).wait_for()
        await page.locator('#code-patch-apply').click()
        await page.locator('#code-patch-undo').wait_for()
        await page.locator('#publish-pr-btn').click()
        await page.locator('#pub-commit').fill('Update brief')
        await page.locator('#pub-submit').click()
        await page.locator('#pub-status').get_by_text('https://github.com/example/repo/pull/99', exact=False).wait_for()
        await page.locator('#modal-close-btn').click()
        await page.screenshot(path=str(OUTPUT / 'code.png'))
        await page.locator('#code-patch-undo').click()
        await page.locator('#code-patch-undo').wait_for(state='hidden')
        await page.locator('[data-view=tools]').click()
        await page.locator('[data-open-preset="study"]').wait_for()
        apps_hub_text = await page.locator('#view-tools').inner_text()
        for jargon in ('GGUF', 'Safetensors', 'loopback', 'Ken Burns', 'FFmpeg'):
            assert jargon not in apps_hub_text, (jargon, apps_hub_text)
        await page.locator('#image-connect').click()
        await page.get_by_text('Image server connected. Choose a checkpoint and output folder.',exact=True).wait_for()
        await page.locator('#image-prompt').fill('A lakeside cabin')
        await page.locator('#image-generate').click()
        await page.get_by_text('succeeded',exact=True).wait_for()
        await page.wait_for_function("document.querySelector('.generated-image')?.naturalWidth > 0")
        await page.locator('#view-tools').evaluate('(el) => { el.scrollTop = 0; }')
        await page.screenshot(path=str(OUTPUT / 'tools.png'))
        await page.locator('#tool-video-open').click()
        await page.locator('#video-render').wait_for()
        await page.locator('#video-render').click()
        await page.locator('[data-load-storyboard]').first.wait_for()
        await page.screenshot(path=str(OUTPUT / 'tools-video.png'))
        await page.locator('#tool-carousels-open').click()
        await page.locator('#slides-about').fill('Weekend trip to Porvoo')
        await page.locator('#slides-build').click()
        await page.locator('#slides-preview-frame').wait_for()
        await page.screenshot(path=str(OUTPUT / 'tools-photo-slides.png'))
        await page.locator('#tool-images-open').click()
        await page.locator('#image-prompt').wait_for()
        assert await page.locator('.topbar').evaluate('(el) => el.getBoundingClientRect().top') >= 0
        # Verify local HTML assets are embedded and traversal/external URLs are untouched.
        result=await page.evaluate('''async () => {
          const {inlineWorkspaceImages}=await import('/viewers.js');
          const seen=[];
          const html=await inlineWorkspaceImages('<img src="assets/a.png"><img src="../escape.png"><img src="https://host/a.png">','index.html',async path=>{seen.push(path);return 'data:image/png;base64,test';});
          return {seen,html};
        }''')
        assert result['seen']==['assets/a.png'],result
        assert 'data:image/png;base64,test' in result['html']

        await page.locator('[data-view=library]').click()
        await page.locator('.library-card').first.wait_for()
        await page.screenshot(path=str(OUTPUT / 'library.png'))
        await page.locator('[data-view=connectors]').click()
        await page.locator('#conn-pick-ws').wait_for()
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='refresh_mcp_servers')")
        await page.locator('[data-view=models]').click()
        models_text = await page.locator('#view-models').inner_text()
        for jargon in ('GGUF', 'Q4', 'Q2_0', 'OpenAI', 'llama-server'):
            assert jargon not in models_text, (jargon, models_text)
        # Discover tab fetches trending models and Scan ~/Models invokes scanner command.
        await page.locator('[data-mfilter="discover"]').click()
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='discover_models_cmd')")
        await page.locator('#view-models').get_by_text('Ternary-Bonsai-2.5-27B', exact=True).wait_for()
        await page.locator('#ws-scan-models').click()
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='scan_user_models_folder')")
        await page.locator('[data-mfilter="all"]').click()
        await page.locator('#ws-ext-url').evaluate('(el) => { el.closest("details").open = true; }')
        await page.locator('#ws-ext-url').fill('https://models.example.com/v1')
        await page.locator('#ws-ext-name').fill('shared-qwen')
        await page.locator('#ws-ext-key').fill('sk-smoke-remote')
        await page.locator('#ws-ext-notice').get_by_text('Leaves this computer', exact=False).wait_for()
        await page.locator('#ws-ext-notice').get_by_text('may incur provider cost', exact=False).wait_for()
        await page.locator('#ws-ext-remote').check()
        await page.locator('#ws-ext-use').click()
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='set_external_endpoint'&&c.args.confirmRemote===true&&c.args.apiKey==='sk-smoke-remote')")
        await page.locator('#model-pill-label').get_by_text('shared-qwen · Leaves this computer · may incur provider cost', exact=True).wait_for()
        await page.locator('[data-view=chat]').click()
        await page.locator('#telemetry-left').get_by_text('Leaves this computer', exact=False).wait_for()
        # Switcher lets you jump back to built-in local AI in one click.
        await page.locator('#model-pill-btn').click()
        await page.locator('#switcher-discover').wait_for()
        await page.locator('#switcher-use-builtin').click()
        await page.locator('#model-pill-label').get_by_text('Test local model', exact=False).wait_for()
        await page.locator('[data-view=models]').click()
        await page.locator('#models-setup-btn').click()
        await page.locator('#welcome-existing').click()
        existing_text = await page.locator('#welcome-overlay').inner_text()
        assert 'OpenAI' not in existing_text and 'Ollama' in existing_text and 'LM Studio' in existing_text, existing_text
        await page.locator('#welcome-back').click()
        await page.locator('#welcome-start').wait_for()
        await page.screenshot(path=str(OUTPUT / 'setup.png'))
        await page.locator('#welcome-start').click()
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='setup_local_ai')")
        await page.evaluate("window.__test.finishSetup()")
        await page.locator('#welcome-overlay').wait_for(state='hidden')
        # Damaged model shows Repair button and recovers cleanly.
        await page.evaluate("window.__test.includeDamaged = true")
        await page.locator('[data-view=chat]').click()
        await page.locator('[data-view=models]').click()
        await page.locator('[data-repair-model="broken-9b"]').wait_for()
        await page.locator('[data-repair-model="broken-9b"]').click()
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='download_catalog_model'&&c.args.modelId==='broken-9b')")
        assert not errors,errors
        await first_run(browser, port)
        await second_launch(browser, port)
        sample = await new_page(browser, port)
        await sample.locator('[data-view=work]').click()
        await sample.get_by_role('button', name='Try with sample files').click()
        await sample.locator('#workspace-pill-label').get_by_text('Sample Work').wait_for()
        assert await sample.evaluate("window.__test.calls.some(c=>c.cmd==='set_workspace_folder' && c.args.path==='/test/Sample Work' && c.args.sessionId)")
        await sample.close()
        limited = await new_page(browser, port)
        await limited.evaluate("window.__test.codeExecution = {can_run_commands:false,sandbox:'none',notice:'Shell commands are off on this Mac until a confined Keel CLI is installed. You can still inspect files and propose edits.'}")
        await limited.locator('[data-view=code]').click()
        await limited.locator('#harness-pill-label').get_by_text('Engine: Inspect & propose', exact=True).wait_for()
        await limited.locator('#code-boundary-notice').get_by_text('inspect files and propose edits', exact=False).wait_for()
        await limited.close()
        paused = await new_page(browser, port, first_run=True)
        await paused.locator('#welcome-size').click()
        await paused.locator('#welcome-start').click()
        await paused.get_by_role('button', name='Pause download').click()
        await paused.get_by_role('heading', name='Continue when you’re ready.').wait_for()
        await paused.screenshot(path=str(OUTPUT / 'first-run-paused.png'))
        await paused.get_by_role('button', name='Resume download').click()
        await paused.wait_for_function("window.__test.calls.filter(c=>c.cmd==='setup_local_ai' && c.args.modelId==='bonsai-4b').length === 2")
        await paused.evaluate('window.__test.finishSetup()')
        await paused.locator('#welcome-overlay').wait_for(state='hidden')
        assert not paused.errors, paused.errors
        await paused.close()
        print('PASS: first-run welcome and setup (progress, queued first question, error and skip), launch wake-up, attachments, source preview, saved preferences, projects, thread rename/pin/search, Chat/Work/Code, file approval, Library, Images, safe HTML previews; no browser errors.')
        await browser.close()

if __name__ == '__main__':
    server = ThreadingHTTPServer(('127.0.0.1', 0), partial(SimpleHTTPRequestHandler, directory=str(ROOT / 'ui')))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        asyncio.run(main(server.server_port))
        print(f'Screenshots: {OUTPUT}')
    finally:
        server.shutdown()
