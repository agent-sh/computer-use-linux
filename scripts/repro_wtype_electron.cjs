// Manual Hyprland/Electron input reproduction; uses synthetic text in its own
// window and temporarily takes keyboard focus. See tests/manual/ for commands.
const { app, BrowserWindow, Menu } = require('electron');
const { spawn, execFileSync } = require('node:child_process');
const fs = require('node:fs');
const output = process.env.CUL_QA_OUTPUT;
const modes = (process.env.CUL_QA_MODES || 'argv,stdin').split(',');
if (!output || modes.some(mode => !['argv', 'stdin', 'mcp', 'mcp-password'].includes(mode)) ||
    (modes.some(mode => mode.startsWith('mcp')) && !process.env.CUL_QA_BINARY)) {
  console.error('Set CUL_QA_OUTPUT; modes: argv,stdin,mcp,mcp-password. MCP mode also needs CUL_QA_BINARY.');
  app.exit(2);
}
const sentence = "Türkçe giriş: PostgreSQL (W, g, m, v, f, ş, ü) / 0123456789 - ; 'İstanbul'. ";
const text = ("Bu bir yazı denemesidir. ".repeat(5) + sentence.repeat(50)).slice(0, 3000);
let mcp;
let rpc;
async function startMcp() {
  mcp = spawn(process.env.CUL_QA_BINARY, ['mcp'], {env: {...process.env, COMPUTER_USE_LINUX_INDICATOR: '0'}, stdio: ['pipe', 'pipe', 'pipe']});
  mcp.stderr.on('data', () => {});
  const pending = new Map();
  let buffer = '', id = 0;
  mcp.stdout.on('data', data => {
    buffer += data;
    let at;
    while ((at = buffer.indexOf('\n')) !== -1) {
      const line = buffer.slice(0, at); buffer = buffer.slice(at + 1);
      let response;
      try { response = JSON.parse(line); }
      catch { console.error('Invalid MCP JSON response'); mcp.kill(); app.exit(1); return; }
      if (pending.has(response.id)) { const done = pending.get(response.id); pending.delete(response.id); done(response); }
    }
  });
  rpc = (method, params) => new Promise((resolve, reject) => {
    const requestId = ++id;
    const timer = setTimeout(() => { pending.delete(requestId); reject(new Error('MCP request timed out')); }, 90000);
    pending.set(requestId, response => { clearTimeout(timer); if (response.error) reject(new Error(JSON.stringify(response.error))); else resolve(response.result); });
    mcp.stdin.write(JSON.stringify({jsonrpc:'2.0', id:requestId, method, params}) + '\n');
  });
  await rpc('initialize', {protocolVersion:'2024-11-05', capabilities:{}, clientInfo:{name:'isolated-input-qa',version:'1'}});
  mcp.stdin.write(JSON.stringify({jsonrpc:'2.0',method:'notifications/initialized'})+'\n');
  await rpc('tools/call', {name:'get_app_state',arguments:{pid:process.pid,include_screenshot:false}});
}
app.whenReady().then(async () => {
  const win = new BrowserWindow({ width: 900, height: 600, title: 'Computer Use isolated input QA', webPreferences: { contextIsolation: true, nodeIntegration: false } });
  Menu.setApplicationMenu(Menu.buildFromTemplate([{ label: 'Edit', submenu: [{ role: 'copy' }, { role: 'paste' }] }]));
  await win.loadURL('data:text/html;charset=utf-8,' + encodeURIComponent(`<!doctype html><meta charset="utf-8"><title>Computer Use isolated input QA</title><h1>Computer Use isolated input QA</h1><textarea id="input" style="width:95%;height:400px"></textarea><script>window.events=[]; for(const kind of ['keydown','keyup'])document.addEventListener(kind,e=>events.push({kind,key:e.key,code:e.code,alt:e.altKey,ctrl:e.ctrlKey,shift:e.shiftKey})); input.focus();</script>`));
  const results = [];
  try {
    for (const mode of modes) {
      win.focus();
      await new Promise(r => setTimeout(r, 800));
      const client = JSON.parse(execFileSync('hyprctl', ['clients', '-j'], { encoding: 'utf8' })).find(c => c.pid === process.pid);
      if (!client || !/^0x[0-9a-f]+$/i.test(client.address)) throw new Error('Cannot locate test window');
      execFileSync('hyprctl', ['dispatch', `hl.dsp.focus({ window = "address:${client.address}" })`]);
      const secret = mode === 'mcp-password';
      await win.webContents.executeJavaScript(`document.getElementById('input').outerHTML = ${JSON.stringify(secret ? '<input id="input" type="password" style="width:95%">' : '<textarea id="input" style="width:95%;height:400px"></textarea>')}; input.value=''; events=[]; input.focus();`);
      const requested = secret ? 'CUL-synthetic-password-şü-123!' : text;
      const expected = secret ? '' : requested;
      await new Promise(r => setTimeout(r, 300));
      const focused = JSON.parse(execFileSync('hyprctl', ['activewindow', '-j'], { encoding: 'utf8' }));
      if (focused.pid !== process.pid) throw new Error('Test window is not focused; refusing to inject text');
      let stderr = '', code, response;
      if (mode.startsWith('mcp')) {
        if (!mcp) await startMcp();
        response = await rpc('tools/call', {name:'type_text',arguments:{pid:process.pid,text:requested}});
        const payload = response.structuredContent || JSON.parse(response.content.find(c => c.type === 'text').text);
        code = !response.isError && (secret ? payload.ok === false && payload.message.includes('password fields') : payload.ok === true) ? 0 : 1;
      } else {
        const args = mode === 'stdin' ? ['-'] : ['--', text];
        const child = spawn('wtype', args, { stdio: ['pipe', 'pipe', 'pipe'] });
        child.stderr.on('data', chunk => stderr += chunk);
        const status = new Promise((resolve, reject) => {
          const timer = setTimeout(() => { child.kill(); reject(new Error('wtype timed out')); }, 90000);
          child.on('exit', code => { clearTimeout(timer); resolve(code); });
          child.on('error', error => { clearTimeout(timer); reject(error); });
          child.stdin.on('error', error => { clearTimeout(timer); child.kill(); reject(error); });
        });
        child.stdin.end(mode === 'argv' ? undefined : text);
        code = await status;
      }
      await new Promise(r => setTimeout(r, 500));
      const actual = await win.webContents.executeJavaScript('({text:input.value, events})');
      const expectedChars = [...expected], actualChars = [...actual.text];
      let firstDifference = 0;
      while (firstDifference < Math.min(expectedChars.length, actualChars.length) &&
             expectedChars[firstDifference] === actualChars[firstDifference]) firstDifference++;
      if (actual.text === expected) firstDifference = -1;
      const result = { mode, exitCode: code, response, expectedLength: [...expected].length, actualLength: [...actual.text].length, match: actual.text === expected && (!secret || actual.events.length === 0), firstDifference, stderr, modifierEvents: actual.events.filter(e => e.alt || e.ctrl), expected, actual: actual.text, events: actual.events };
      results.push(result);
      fs.writeFileSync(output, JSON.stringify({electron: process.versions.electron, chromium: process.versions.chrome, results}, null, 2));
      console.log(JSON.stringify({mode, code, expectedLength: result.expectedLength, actualLength: result.actualLength, match: result.match, firstDifference, modifierEventCount: result.modifierEvents.length}));
    }
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  } finally { if (mcp) mcp.kill(); }
  app.exit(process.exitCode || (results.every(result => result.match && result.exitCode === 0) ? 0 : 1));
});
