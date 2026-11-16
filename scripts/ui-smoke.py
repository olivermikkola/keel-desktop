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
const listeners = {}, sessions = [], jobs = [];
let cfg = {selected_model_id:'test',external_base_url:null,external_model_name:null,image_endpoint:null,recent_workspaces:[],harness_mode:'keel', custom_model_paths:{}};
let runtime = {state:'stopped',active_model_id:null,base_url:'http://127.0.0.1:18180/v1'};
const presets = ['chat','general','code','websites','docs','sheets','study','slides'].map(id => ({id,title:({chat:'Chat',general:'Work',code:'Code'})[id] || id[0].toUpperCase()+id.slice(1),summary:'A useful local workspace',starters:['Help me get started'],tools:[]}));
let files = [], fileContent = {};
const emit = (event,payload) => (listeners[event] || []).forEach(fn => fn({payload}));
window.__test = {calls:[]};
window.__TAURI__ = {event:{listen:async (event,fn) => { (listeners[event] ||= []).push(fn); }},core:{invoke:async (cmd,args={}) => {
 window.__test.calls.push({cmd,args});
 let result;
 switch(cmd) {
 case 'get_bootstrap_state': result={version:'0.1.0',config:cfg, runtime,models:[{id:'test',name:'Test local model',installed:true,quantization:'Q4',bytes:1000000000,detail:'Fast local model',tier:'fast'}],sessions:sessions.map(s=>({...s})),presets,harnesses:[{id:'keel',name:'Keel Harness'}],mcp_servers:[]};break;
 case 'create_session': result={id:crypto.randomUUID(),title:'New '+args.preset+' chat',app_preset:args.preset,workspace_path:args.workspacePath,messages:[],staged_edits:[],updated_at:Date.now()/1000};sessions.unshift(result);break;
 case 'load_session_cmd':result=sessions.find(s=>s.id===args.id);break;
 case 'list_workspace_files_cmd':result=files;break;
 case 'list_image_jobs':result=jobs;break;
 case 'pick_workspace_folder':result='/test/project';break;
 case 'set_workspace_folder':cfg.recent_workspaces=['/test/project'];result='/test/project';break;
 case 'start_model':runtime={...runtime,state:'running',active_model_id:'test'};result=runtime;break;
 case 'stop_runtime':runtime={...runtime,state:'stopped',active_model_id:null};result=runtime;break;
 case 'connect_image_server':cfg.image_endpoint=args.endpoint;result=['Local checkpoint'];break;
 case 'read_workspace_png':result=''' + json.dumps(PNG) + r''';break;
 case 'read_workspace_file_raw':result=fileContent[args.relPath] || '# Local document';break;
 case 'send_message': {
 const s=sessions.find(s=>s.id===args.input.session_id);
 s.messages.push({id:'u'+Date.now(),role:'user',content:args.input.prompt});
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
 case 'start_image_job': {
 const job={id:crypto.randomUUID(),request:args.request,workspace_path:args.workspacePath,status:'running',artifact_path:null,created_at:Date.now()/1000};jobs.unshift(job);result=job;
 setTimeout(()=>{job.status='succeeded';job.artifact_path='assets/keel-test.png';files.push(job.artifact_path);emit('keel://image-job',job);},250);break;}
 default: throw new Error('Unexpected command '+cmd);
 }
 return structuredClone(result);
}}};
})();'''

async def main(port):
    async with async_playwright() as p:
        browser = await p.chromium.launch(executable_path=os.environ.get('CHROMIUM_BIN') or shutil.which('chromium'), args=['--no-sandbox'])
        page = await browser.new_page(viewport={'width':1440,'height':1000})
        errors=[]
        page.on('pageerror', lambda e:errors.append(str(e)))
        await page.add_init_script(MOCK)
        page.set_default_timeout(10000)
        await page.goto(f'http://127.0.0.1:{port}')
        await page.get_by_role('heading', name='Chat', exact=True).wait_for()
        assert await page.locator('#workspace-pill-btn').is_hidden()
        await page.locator('#prompt-input').fill('Hello Keel')
        await page.locator('#send-btn').click()
        await page.get_by_text('Load a model or connect an existing server in Models.',exact=True).wait_for()
        assert not await page.evaluate("window.__test.calls.some(c=>c.cmd==='send_message')")
        await page.locator('[data-view=models]').click()
        await page.locator('[data-start-model]').click()
        await page.get_by_text('• Loaded',exact=True).wait_for()
        await page.locator('[data-view=chat]').click()
        await page.locator('#send-btn').click()
        await page.get_by_text('Ready to help.',exact=True).wait_for()
        assert await page.locator('#viewer-pane').is_hidden()
        await page.screenshot(path=str(OUTPUT / 'chat.png'))
        await page.locator('[data-view=work]').click()
        await page.get_by_role('heading',name='Work',exact=True).wait_for()
        await page.locator('#workspace-pill-btn').click()
        await page.locator('#prompt-input').fill('Make a project brief')
        await page.locator('#send-btn').click()
        await page.locator('[data-edit-approve]').wait_for()
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
        await page.screenshot(path=str(OUTPUT / 'code.png'))
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
        assert not errors,errors
        print('PASS: Chat model preflight, load/status, conversation, Work folder + staged approval, Code tools/viewer, optional Images connect/generate/preview, safe HTML asset embedding; no browser errors.')
        await browser.close()

if __name__ == '__main__':
    server = ThreadingHTTPServer(('127.0.0.1', 0), partial(SimpleHTTPRequestHandler, directory=str(ROOT / 'ui')))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        asyncio.run(main(server.server_port))
        print(f'Screenshots: {OUTPUT}')
    finally:
        server.shutdown()
