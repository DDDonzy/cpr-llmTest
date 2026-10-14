import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { buildDraftTasks, selectionKey, selectedIndexes } from '../frontend/src/selection.ts'
const app=readFileSync(new URL('../frontend/src/App.vue',import.meta.url),'utf8')
const css=readFileSync(new URL('../frontend/src/styles.css',import.meta.url),'utf8')
const template=app.slice(app.indexOf('<template>'))
test('native controls own all interactive UI and answer is last result column',()=>{
 assert.doesNotMatch(template,/<(?:button|input|select|textarea|table|dialog)\b/i)
 assert.doesNotMatch(app,/Todo List|maxOutputTokens/)
 assert.match(app,/{key:'result',label:'答案'/)
 assert.match(template,/class="answer-text"/)
 assert.match(template,/<ZCheckbox v-for="a in visibleAccounts"/)
 assert.match(template,/<ZCheckbox v-for="m in visibleModels"/)
})
test('reference board has account / model / question, tasks + run, separate results',()=>{
 for(const panel of ['account','model','question','tasks','run','results'])assert.match(template,new RegExp(`data-testid="${panel}-panel"`))
 assert.match(css,/\.test-board\s*\{[^}]*repeat\(3, minmax\(0, 1fr\)\)/s)
 assert.match(css,/\.tasks-card\s*\{\s*grid-column:\s*span 2/)
 assert.ok(template.indexOf('data-testid="results-panel"')>template.indexOf('data-testid="run-panel"'))
})
test('select / number roots stretch even with no selection and responsive columns shrink',()=>{
 for(const [control] of template.matchAll(/<Z(?:Select|InputNumber)\b[^>]*>/g))assert.match(control,/class="field-control"/)
 assert.match(css,/\.field-control\s*\{[^}]*width:\s*100%;[^}]*min-width:\s*0;/s)
 assert.match(css,/@media\s*\(max-width:\s*900px\)/)
 assert.match(css,/@media\s*\(max-width:\s*560px\)/)
 assert.match(template,/collapse-tags-tooltip/)
})
test('combinations default all selected, partial deselection maps exact server indexes',()=>{
 const a=[{account_id:'a',name:'A',email:null,provider_id:'openai',enabled:true},{account_id:'b',name:'B',email:null,provider_id:'openai',enabled:true}]
 const rows=buildDraftTasks(a,['a','b'],['m1','m2'],['low','high'])
 assert.equal(rows.length,8)
 assert.deepEqual(selectedIndexes(rows,new Set()),[0,1,2,3,4,5,6,7])
 const excluded=new Set(rows.filter(r=>![1,6].includes(r.index)).map(r=>r.key))
 assert.deepEqual(selectedIndexes(rows,excluded),[1,6])
 assert.deepEqual(selectedIndexes(rows,new Set(rows.map(r=>r.key))),[])
 assert.deepEqual([rows[1].accountId,rows[1].model,rows[1].effort],['a','m1','high'])
 // Deselecting a stable tuple survives unrelated additions / model reorder.
 const newer=buildDraftTasks(a,['a','b'],['m2','m1','m3'],['low','high'])
 const omitted=new Set([rows[1].key])
 assert.equal(newer.filter(r=>!selectedIndexes(newer,omitted).includes(r.index))[0].key,rows[1].key)
 assert.match(app,/selectedTaskIndexes:\[\.\.\.selectedTaskIndexes.value\]/)
 assert.match(template,/:disabled="!canStart"/)
})
test('tuple keys cannot collide and preview is bounded to 500 rows',()=>{
 assert.notEqual(selectionKey('a|b','c','d'),selectionKey('a','b|c','d'))
 assert.equal(buildDraftTasks([],['a'],Array.from({length:1000},(_,i)=>`m${i}`),['high']).length,500)
})
test('preview fixtures are never shipped and presets namespace is unchanged',()=>{
 const manifest=JSON.parse(readFileSync(new URL('../plugin.json',import.meta.url),'utf8'))
 assert.deepEqual(Object.keys(manifest.resources).sort(),['web/app.css','web/app.js','web/index.html'])
 assert.equal(manifest.state[0].namespace,'prompt-presets')
 assert.equal(manifest.state[0].schemaVersion,1)
})

test('requested defaults, no plugin concurrency ceiling, no question refresh',()=>{
 assert.match(app,/concurrency=ref\(10\),timeout=ref\(180\)/)
 assert.doesNotMatch(template,/:max="(?:8|100)"/)
 assert.doesNotMatch(app,/RefreshCw/)
 assert.match(app,/api\/run-batch/)
 assert.match(template,/有效测试期限约/)
})
test('preset scrolling is native and native dialog uses visible sandbox intersection',()=>{
 assert.match(template,/<ZScrollbar max-height="100px" class="preset-scroll">/)
 assert.doesNotMatch(css,/\.preset-preview\s*\{[^}]*overflow:\s*auto/s)
 const hook=readFileSync(new URL('../frontend/src/dialog-viewport.ts',import.meta.url),'utf8')
 assert.match(hook,/new IntersectionObserver/)
 assert.match(hook,/intersectionRect/)
 assert.doesNotMatch(hook,/window\.(?:parent|top)\.(?:document|innerHeight)/)
 assert.match(css,/body\[data-dialogue-viewport\] > \[role="presentation"\]/)
 assert.match(css,/--dialogue-visible-height/)
})
