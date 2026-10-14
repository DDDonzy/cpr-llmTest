import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { resolveTheme, DEFAULT_THEME_COLOR, DEFAULT_CUSTOM_THEME_COLOR } from '../frontend/node_modules/@codex-proxy/ui/dist/theme/index.js'
const root=new URL('../web/',import.meta.url)
const server=createServer(async(req,res)=>{
 try {
  const u=new URL(req.url,'http://127.0.0.1');
  if(u.pathname==='/embedded') {
   res.setHeader('Content-Type','text/html; charset=utf-8');res.setHeader('Cache-Control','no-store');
   return res.end(`<!doctype html><html><head><meta charset="utf-8"><title>沙箱可见区域验证</title><style>body{margin:0;background:#0b1018;color:#ced9e7;font:14px sans-serif}aside{position:fixed;left:0;top:0;width:200px;height:100vh;background:#111b2b;padding:24px;box-sizing:border-box}main{margin-left:220px;padding:0 20px}header{height:112px;display:flex;align-items:center}iframe{border:0;width:100%;height:1800px}footer{height:400px}@media(max-width:600px){aside{display:none}main{margin-left:0;padding:0 16px}}</style></head><body><aside>本地沙箱验证<br>不连接真实上游</aside><main><header><h1>沙箱内弹窗可见区域测试</h1></header><iframe title="测试插件" sandbox="allow-scripts" src="/?state=interactive&longPreset=1"></iframe><footer>用于验证父页面滚动后的弹窗居中</footer></main></body></html>`)
  }
  if(u.pathname==='/fixture.js') {
   res.setHeader('Content-Type','text/javascript; charset=utf-8');res.setHeader('Cache-Control','no-store');
   return res.end(await readFile(new URL('./ui-fixture.js',import.meta.url)))
  }
  if(!['/','/index.html','/app.js','/app.css'].includes(u.pathname)){res.writeHead(404);return res.end()}
  const name=u.pathname==='/'?'index.html':u.pathname.slice(1)
  let data=await readFile(new URL(name,root))
  if(name==='index.html'){
   const theme=resolveTheme(u.searchParams.get('theme')==='light'?'light':'dark',DEFAULT_THEME_COLOR,DEFAULT_CUSTOM_THEME_COLOR)
   const css=Object.entries(theme.tokens).map(([name,value])=>`${name}:${value}`).join(';')
   data=Buffer.from(data.toString().replace('<head>','<head><style>:root{'+css+'}</style><script src="fixture.js"></script>'))
  }
  res.setHeader('Content-Type',name.endsWith('.html')?'text/html; charset=utf-8':name.endsWith('.js')?'text/javascript; charset=utf-8':'text/css; charset=utf-8');res.setHeader('Cache-Control','no-store');res.end(data)
 }catch(e){res.writeHead(500);res.end('Fixture preview unavailable')}
});server.listen(18947,'127.0.0.1',()=>console.log('UI fixture preview: http://127.0.0.1:18947'))
