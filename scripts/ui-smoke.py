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
 case 'get_bootstrap_state': result={version:'0.1.0',config:cfg, runtime,models:[{id:'test',name:'Test local model',installed:true,quantization:'Q4',bytes:1000000000,detail:'Fast local model',tier:'fast'}],sessions:sessions.map(s=>({...s})),projects,presets,harnesses:[{id:'keel',name:'Keel Harness'}],mcp_servers:[]};break;
 case 'create_session': result={id:crypto.randomUUID(),title:'New '+args.preset+' chat',app_preset:args.preset,workspace_path:args.workspacePath,project_id:args.projectId || null,pinned:false,messages:[],staged_edits:[],updated_at:Date.now()/1000};sessions.unshift(result);break;
 case 'load_session_cmd':result=sessions.find(s=>s.id===args.id);break;
 case 'list_workspace_files_cmd':result=files;break;
 case 'list_image_jobs':result=jobs;break;
 case 'get_runtime_status':result=runtime;break;
 case 'pick_documents':result=[{id:crypto.randomUUID(),name:'brief.txt',kind:'txt',text:'Project Harbor budget is 4800 euros.',truncated:false}];break;
 case 'save_personal_context':cfg.personal_context=args.text;result=null;break;
 case 'save_project':{const old=projects.findIndex(p=>p.id===args.project.id);if(old>=0)projects[old]=args.project;else projects.push(args.project);result=projects;break;}
 case 'get_project':result=projects.find(p=>p.id===args.id);break;
 case 'search_threads':result=sessions.filter(s=>(!args.projectId||s.project_id===args.projectId)&&(s.title+' '+s.messages.map(m=>m.content).join(' ')).toLowerCase().includes(args.query.toLowerCase()));break;
 case 'update_thread':{const thread=sessions.find(s=>s.id===args.id);Object.assign(thread,{title:args.title,pinned:args.pinned,project_id:args.projectId});result=thread;break;}
 case 'fork_thread':{const previous=sessions.find(s=>s.id===args.id);const index=previous.messages.findIndex(m=>m.id===args.beforeMessageId);result={...structuredClone(previous),id:crypto.randomUUID(),messages:previous.messages.slice(0,index),staged_edits:[]};sessions.unshift(result);break;}
 case 'list_library':result=files.map(path=>({id:path,name:path,path,kind:path.split('.').pop(),workspace_path:'/test/project',project_id:projects[0]?.id || null,updated_at:Date.now()/1000}));break;
 case 'get_setup_plan':result={memory_bytes:17179869184,runtime_path:null,runtime_asset:{bytes:20000000},recommended_model:{id:'test',name:'Recommended model',detail:'A balanced local assistant',bytes:1000000000}};break;
 case 'setup_local_ai':runtime={...runtime,state:'running',active_model_id:'test'};result=runtime;break;
 case 'pick_workspace_folder':result=window.__test.cancelPicker ? null : '/test/project';break;
 case 'prepare_sample_workspace':result='/test/Sample Work';break;
 case 'set_workspace_folder': {
 const s=sessions.find(s=>s.id===args.sessionId); if(s) s.workspace_path=args.path;
 cfg.recent_workspaces=[args.path];result=args.path;break;}
 case 'start_model':runtime={...runtime,state:'running',active_model_id:'test'};result=runtime;break;
 case 'stop_runtime':runtime={...runtime,state:'stopped',active_model_id:null};result=runtime;break;
 case 'set_external_endpoint':cfg.external_base_url=args.baseUrl;cfg.external_model_name=args.modelName || null;runtime={state:'external',active_model_id:null,base_url:args.baseUrl};result=runtime;break;
 case 'connect_image_server':cfg.image_endpoint=args.endpoint;result=['Local checkpoint'];break;
 case 'read_workspace_png':result=''' + json.dumps(PNG) + r''';break;
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
 case 'start_image_job': {
 const job={id:crypto.randomUUID(),request:args.request,workspace_path:args.workspacePath,status:'running',artifact_path:null,created_at:Date.now()/1000};jobs.unshift(job);result=job;
 setTimeout(()=>{job.status='succeeded';job.artifact_path='assets/keel-test.png';files.push(job.artifact_path);emit('keel://image-job',job);},250);break;}
 default: throw new Error('Unexpected command '+cmd);
 }
 return structuredClone(result);
}}};
})();'''


FIRST_RUN_LAYER = r'''(() => {
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
 if (cmd==='start_model' && t.holdStart) {
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
    return script + FIRST_RUN_LAYER + f"window.__test.firstRun={'true' if first_run else 'false'};window.__test.holdStart={'true' if hold_start else 'false'};"

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
    await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='start_model')")
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
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='start_model')")
        await page.locator('#model-pill-label').get_by_text('Test local model', exact=False).wait_for()
        await page.locator('#prompt-input').fill('Hello Keel')
        await page.locator('#send-btn').click()
        await page.get_by_text('Ready to help.',exact=True).wait_for()
        assert await page.locator('#viewer-pane').is_hidden()
        await page.screenshot(path=str(OUTPUT / 'chat.png'))
        # Attach a source, inspect it, then save explicit cross-conversation preferences.
        await page.locator('#attach-docs-btn').click()
        await page.locator('[data-pending-source]').wait_for()
        await page.locator('#prompt-input').fill('Summarize the attached brief')
        await page.locator('#send-btn').click()
        await page.locator('[data-message-source]').wait_for()
        await page.locator('[data-message-source]').click()
        await page.locator('.document-text').get_by_text('Project Harbor budget is 4800 euros.',exact=True).wait_for()
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
        await page.screenshot(path=str(OUTPUT / 'code.png'))
        await page.locator('#code-patch-undo').click()
        await page.locator('#code-patch-undo').wait_for(state='hidden')
        await page.locator('[data-view=tools]').click()
        await page.locator('#image-connect').click()
        await page.get_by_text('Image server connected. Choose a checkpoint and output folder.',exact=True).wait_for()
        await page.locator('#image-prompt').fill('A lakeside cabin')
        await page.locator('#image-generate').click()
        await page.get_by_text('succeeded',exact=True).wait_for()
        await page.wait_for_function("document.querySelector('.generated-image')?.naturalWidth > 0")
        await page.locator('#view-tools').evaluate('(el) => { el.scrollTop = 0; }')
        await page.screenshot(path=str(OUTPUT / 'tools.png'))
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
        await page.locator('[data-view=models]').click()
        await page.locator('#ws-ext-url').evaluate('(el) => { el.closest("details").open = true; }')
        await page.locator('#ws-ext-url').fill('https://models.example.com/v1')
        await page.locator('#ws-ext-name').fill('shared-qwen')
        await page.locator('#ws-ext-notice').get_by_text('server you connect here', exact=False).wait_for()
        await page.locator('#ws-ext-use').click()
        await page.locator('#model-pill-label').get_by_text('shared-qwen · Remote', exact=True).wait_for()
        await page.locator('[data-view=chat]').click()
        await page.locator('#telemetry-left').get_by_text('External model server', exact=False).wait_for()
        await page.locator('[data-view=models]').click()
        await page.locator('#models-setup-btn').click()
        await page.locator('#welcome-start').wait_for()
        await page.screenshot(path=str(OUTPUT / 'setup.png'))
        await page.locator('#welcome-start').click()
        await page.wait_for_function("window.__test.calls.some(c=>c.cmd==='setup_local_ai')")
        await page.evaluate("window.__test.finishSetup()")
        await page.locator('#welcome-overlay').wait_for(state='hidden')
        assert not errors,errors
        await first_run(browser, port)
        await second_launch(browser, port)
        sample = await new_page(browser, port)
        await sample.locator('[data-view=work]').click()
        await sample.get_by_role('button', name='Try with sample files').click()
        await sample.locator('#workspace-pill-label').get_by_text('Sample Work').wait_for()
        assert await sample.evaluate("window.__test.calls.some(c=>c.cmd==='set_workspace_folder' && c.args.path==='/test/Sample Work' && c.args.sessionId)")
        await sample.close()
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
