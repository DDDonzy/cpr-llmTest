import importlib.util,pathlib,json,struct,unittest
spec=importlib.util.spec_from_file_location('h',pathlib.Path(__file__).with_name('ipc-harness.py'));h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)
def batch(facts):
 m=json.dumps({'facts':facts}).encode();event=b'GPE2'+struct.pack('>III',len(m),0,0)+m
 return b'HME1'+struct.pack('>II',1,len(event))+event
class Harness(h.Harness):
 def __init__(self,fail=False,mismatch=False):self.executions=[];self.fail=fail;self.mismatch=mismatch;super().__init__()
 def callback(self,meta,payload):
  method=meta['method'];p=meta['params'];self.callbacks.append(method);result={};body=b''
  if method=='host.state.get':
   assert p['namespace']=='prompt-presets' and p['key']=='document'
   result={'record':None if self.doc is None else {'value':self.doc,'version':self.version,'schema_version':1}}
  elif method=='host.state.put':
   assert p['namespace']=='prompt-presets' and p['key']=='document'
   if p['expected_version']!=self.version:
    self.send({'type':'error','id':meta['id'],'error':{'code':'conflict','message':'stale','send_state':'not_sent'}});return
   self.doc=p['value'];self.version=(self.version or 0)+1;self.writes.append(p);result={'version':self.version}
  elif method=='host.data.accounts.list':body=json.dumps({'schema_version':1,'next_cursor':None,'accounts':[{'account_id':id,'provider_id':'openai','name':id,'email':id+'@example.test','group_ids':[],'enabled':True,'updated_at_ms':0} for id in ['a','b']]}).encode()
  elif method=='host.keys.list':result={'keys':[{'id':'key','name':'TEST','enabled':True}],'next_cursor':None}
  elif method=='host.models.list':result={'models':['m1','m2']}
  elif method=='host.model.execute_stream':
   self.executions.append((p,json.loads(payload)));assert p['protocol']=='openai';assert p['account_id'] in ['a','b'];assert p['client_key_id']=='key';assert p['provider']=='openai'
   if self.fail:self.send({'type':'error','id':meta['id'],'error':{'code':'upstream','message':'Bearer SECRET','send_state':'sent','details':{'kind':'upstream_unavailable','provider':{'raw_upstream_error':'SECRET'}}}});return
   result={'request_id':'req_'+p['account_id'],'stream':'stream_'+p['account_id']}
  elif method=='host.model.stream_read':result={'events':1,'end':True};body=batch([{'type':'text_delta','index':0,'text':'2'},{'type':'usage','usage':{'input_tokens':8,'output_tokens':1}},{'type':'completed','id':'response_fixture','model':'m1','reason':'stop'}])
  elif method=='host.http.dispatch':
   assert p['method']=='GET' and p['uri'].startswith('/api/admin/usage/records/detail?id=req_')
   id=p['uri'].split('=')[-1];account='wrong' if self.mismatch else id[4:];data={'requestId':id,'accountId':account,'attempts':[{'accountId':account}],'attemptsComplete':True,'reasoningEffort':'high','logicalOutcome':'success'}
   self.bodies['audit']=json.dumps({'code':200,'data':data}).encode();result={'status':200,'version':'HTTP/1.1','headers':[],'body':{'kind':'handle','handle':'audit'},'response':None,'session':False}
  elif method=='host.http.body_read':body=self.bodies['audit'];self.bodies['audit']=b'';result={'eof':not bool(body),'trailers':None}
  elif method=='host.http.body_close' or method=='host.model.stream_close':pass
  else:raise AssertionError('unexpected host operation '+method)
  self.send({'type':'result','id':meta['id'],'result':result},body)
def plan(**kw):
 p={'keyId':'key','accounts':['a','b'],'models':['m1','m2'],'efforts':['low','high'],'prompt':'1+1?','concurrency':2,'timeoutSeconds':20};p.update(kw);return p
class Tests(unittest.TestCase):
 def test_matrix_and_no_generation_before_start(self):
  with Harness() as x:
   status,j=x.management('api/plan',plan());self.assertEqual(status,200);self.assertEqual(len(j['tasks']),8);self.assertTrue(j['paused']);self.assertEqual(x.executions,[])
   self.assertEqual(x.management('api/run',{'jobId':j['id'],'taskId':0})[0],409);self.assertEqual(x.executions,[])
   self.assertEqual([(t['accountId'],t['model'],t['effort']) for t in j['tasks']],[ (a,m,e) for a in ['a','b'] for m in ['m1','m2'] for e in ['low','high']])
 def test_selected_subset_only_sends_checked_combinations(self):
  with Harness() as x:
   status,j=x.management('api/plan',plan(selectedTaskIndexes=[1,6]));self.assertEqual(status,200)
   self.assertEqual([(t['id'],t['accountId'],t['model'],t['effort']) for t in j['tasks']],[(0,'a','m1','high'),(1,'b','m2','low')])
   self.assertEqual(j['plan']['selectedTaskIndexes'],[1,6]);self.assertEqual(x.executions,[])
   x.management('api/control',{'jobId':j['id'],'action':'start'})
   for t in j['tasks']:x.management('api/run',{'jobId':j['id'],'taskId':t['id']})
   self.assertEqual([(p['account_id'],b['model'],b['reasoning']['effort']) for p,b in x.executions],[('a','m1','high'),('b','m2','low')])
   self.assertEqual(x.management('api/run',{'jobId':j['id'],'taskId':2})[0],400)
 def test_invalid_selection_never_generates(self):
  with Harness() as x:
   for indexes in [[],[0,0],[8],[-1],[999999999999],[True],['1']]:self.assertEqual(x.management('api/plan',plan(selectedTaskIndexes=indexes))[0],400)
   self.assertEqual(x.executions,[])

 def test_batch_ten_simultaneous_no_duplicates_and_summary_only(self):
  with Harness() as x:
   _,j=x.management('api/plan',plan(accounts=['a'],models=[f'm{i}' for i in range(10)],efforts=['high'],concurrency=10,timeoutSeconds=180));x.management('api/control',{'jobId':j['id'],'action':'start'})
   held=[];orig=x.callback
   def cb(meta,payload):
    if meta['method']=='host.model.stream_read' and len(x.executions)<10:held.append(meta)
    else:orig(meta,payload)
    if len(x.executions)==10 and held:
     for m in held:orig(m,b'')
     held.clear()
   x.callback=cb
   status,out=x.management('api/run-batch',{'jobId':j['id'],'taskIds':list(range(10))});self.assertEqual(status,200);self.assertEqual(len(x.executions),10);self.assertEqual(out['issues'],[])
   self.assertTrue(all(t['status']=='success' for t in out['job']['tasks']));self.assertTrue(all('result' not in t for t in out['job']['tasks']))
   x.management('api/run-batch',{'jobId':j['id'],'taskIds':list(range(10))});self.assertEqual(len(x.executions),10)
   _,t=x.management('api/task',{'jobId':j['id'],'taskId':0});self.assertEqual(t['result']['configuredTimeoutSeconds'],180);self.assertEqual(t['result']['effectiveTimeoutSeconds'],22)
 def test_batch_validation_has_no_side_effects(self):
  with Harness() as x:
   _,j=x.management('api/plan',plan());self.assertEqual(x.management('api/run-batch',{'jobId':j['id'],'taskIds':[0]})[0],409)
   x.management('api/control',{'jobId':j['id'],'action':'start'})
   for ids in [[],[0,0],[0,999]]:self.assertEqual(x.management('api/run-batch',{'jobId':j['id'],'taskIds':ids})[0],400)
   self.assertEqual(x.management('api/run-batch',{'jobId':j['id'],'taskIds':[0,1,2]})[0],429);self.assertEqual(x.executions,[])
 def test_snapshot_exposes_actual_host_deadline(self):
  with Harness() as x:
   _,s=x.management('api/snapshot');self.assertEqual(s['managementTimeoutSeconds'],30)
   for c in [10,100,1000000]:self.assertEqual(x.management('api/plan',plan(concurrency=c,timeoutSeconds=180))[0],200);x.management('api/control',{'jobId':x.management('api/job')[1]['job']['id'],'action':'cancel'})

 def test_execute_verified_no_duplicate(self):
  with Harness() as x:
   _,j=x.management('api/plan',plan());x.management('api/control',{'jobId':j['id'],'action':'start'})
   _,t=x.management('api/run',{'jobId':j['id'],'taskId':0});self.assertEqual(t['status'],'success');self.assertEqual(t['result']['text'],'2');self.assertEqual(t['result']['verification'],'verified');self.assertEqual(x.executions[0][1]['reasoning'],{'effort':'low'})
   x.management('api/run',{'jobId':j['id'],'taskId':0});self.assertEqual(len(x.executions),1)
 def test_default_omits_reasoning(self):
  with Harness() as x:
   _,j=x.management('api/plan',plan(efforts=['default']));x.management('api/control',{'jobId':j['id'],'action':'start'});x.management('api/run',{'jobId':j['id'],'taskId':0});self.assertNotIn('reasoning',x.executions[0][1]);self.assertNotIn('max_output_tokens',x.executions[0][1])
 def test_cancel_pending_and_new_plan(self):
  with Harness() as x:
   _,j=x.management('api/plan',plan());self.assertEqual(x.management('api/plan',plan())[0],409);_,j=x.management('api/control',{'jobId':j['id'],'action':'cancel'});self.assertTrue(all(t['status']=='cancelled' for t in j['tasks']));self.assertEqual(x.executions,[]);self.assertEqual(x.management('api/plan',plan())[0],200)
 def test_error_no_secret_no_retry(self):
  with Harness(fail=True) as x:
   _,j=x.management('api/plan',plan());x.management('api/control',{'jobId':j['id'],'action':'start'});_,t=x.management('api/run',{'jobId':j['id'],'taskId':0});self.assertEqual(t['status'],'error');self.assertNotIn('SECRET',json.dumps(t));x.management('api/run',{'jobId':j['id'],'taskId':0});self.assertEqual(len(x.executions),1)
 def test_mismatch_is_error(self):
  with Harness(mismatch=True) as x:
   _,j=x.management('api/plan',plan());x.management('api/control',{'jobId':j['id'],'action':'start'});_,t=x.management('api/run',{'jobId':j['id'],'taskId':0});self.assertEqual(t['status'],'error');self.assertEqual(t['result']['verification'],'mismatch')
 def test_validation_and_readonly(self):
  with Harness() as x:
   for p in [plan(concurrency=0),plan(accounts=['a','a']),plan(efforts=['secret']),plan(prompt=''),plan(models=['m']*501),plan(accounts=['unknown'])]:self.assertEqual(x.management('api/plan',p)[0],400)
   _,v=x.management('api/snapshot');self.assertEqual(len(v['accounts']),2);self.assertEqual(len(v['keys']),1);self.assertEqual(x.executions,[]);self.assertFalse(any('save' in m or 'state.put' in m for m in x.callbacks))

 def test_concurrency_guard_pause_and_inflight_completion(self):
  with Harness() as x:
   _,j=x.management('api/plan',plan(concurrency=1));x.management('api/control',{'jobId':j['id'],'action':'start'})
   x.seq+=2;runid=x.seq
   x.send({'type':'call','id':runid,'method':'management.handle','context':x.context(runid,'management'),'params':{'method':'POST','path':'api/run','query':'','content_type':'application/json','headers':[]}},json.dumps({'jobId':j['id'],'taskId':0}).encode())
   held=None
   while held is None:
    m,b=x.read()
    if m['type']=='callback' and m['method']=='host.model.stream_read':held=m
    elif m['type']=='callback':x.callback(m,b)
    elif m['type']=='credit':continue
    else:raise AssertionError(m)
   self.assertEqual(x.management('api/run',{'jobId':j['id'],'taskId':1})[0],429)
   self.assertEqual(x.management('api/run',{'jobId':j['id'],'taskId':0})[0],409)
   _,status=x.management('api/control',{'jobId':j['id'],'action':'cancel'})
   self.assertEqual(status['tasks'][0]['status'],'running');self.assertTrue(all(t['status']=='cancelled' for t in status['tasks'][1:]));self.assertEqual(len(x.executions),1)
   x.callback(held,b'')
   while True:
    m,b=x.read()
    if m['type']=='callback':x.callback(m,b)
    elif m['type']=='credit':continue
    elif m['type']=='result' and m['id']==runid:self.assertEqual(json.loads(b)['status'],'success');break
    else:raise AssertionError(m)
   self.assertEqual(len(x.executions),1)
 def test_partial_audit_is_not_complete_verification(self):
  with Harness() as x:
   orig=x.callback
   def cb(meta,payload):
    if meta['method']=='host.http.body_read':x.bodies['audit']=x.bodies['audit'].replace(b'"attemptsComplete": true',b'"attemptsComplete": false')
    orig(meta,payload)
   x.callback=cb
   _,j=x.management('api/plan',plan());x.management('api/control',{'jobId':j['id'],'action':'start'});_,t=x.management('api/run',{'jobId':j['id'],'taskId':0});self.assertEqual(t['result']['verification'],'matched');self.assertFalse(t['result']['attemptsComplete'])
 def test_stale_job_unknown_field_and_no_data_mutation(self):
  with Harness() as x:
   self.assertEqual(x.management('api/plan',plan(url='https://malicious.invalid'))[0],400)
   _,j=x.management('api/plan',plan());self.assertEqual(x.management('api/run',{'jobId':'stale','taskId':0})[0],409)
   self.assertEqual(x.management('api/run',{'jobId':j['id'],'taskId':999})[0],400);self.assertEqual(x.executions,[])


 def test_presets_create_read_update_and_remove(self):
  with Harness() as x:
   self.assertEqual(x.management('api/presets')[1],{'version':None,'presets':[]})
   p={'id':'preset_one','name':'算术','prompt':'1+1?'}
   status,v=x.management('api/presets',{'expectedVersion':None,'presets':[p]});self.assertEqual(status,200);self.assertEqual(v['version'],1)
   self.assertEqual(x.management('api/presets')[1],v)
   p['prompt']='2+2?';status,v=x.management('api/presets',{'expectedVersion':1,'presets':[p]});self.assertEqual(v['presets'][0]['prompt'],'2+2?');self.assertEqual(v['version'],2)
   status,v=x.management('api/presets',{'expectedVersion':2,'presets':[]});self.assertEqual(v['presets'],[]);self.assertEqual(len(x.writes),3);self.assertEqual(x.executions,[])
 def test_preset_cas_conflict_preserves_existing(self):
  with Harness() as x:
   p={'id':'preset_one','name':'保留','prompt':'test'};x.management('api/presets',{'expectedVersion':None,'presets':[p]})
   self.assertEqual(x.management('api/presets',{'expectedVersion':None,'presets':[]})[0],409);self.assertEqual(x.management('api/presets')[1]['presets'],[p]);self.assertEqual(len(x.writes),1)
 def test_invalid_presets_have_no_writes(self):
  with Harness() as x:
   p={'id':'one','name':'name','prompt':'test'}
   bad=[[p,p],[dict(p,name='')],[dict(p,prompt=' ')],[dict(p,id='../secret')],[dict(p,name='x'*65)],[dict(p,prompt='x'*16385)],[dict(p,id=str(i),name=str(i)) for i in range(33)],[dict(p,id=str(i),name=str(i),prompt='x'*16000) for i in range(4)]]
   for ps in bad:self.assertEqual(x.management('api/presets',{'expectedVersion':None,'presets':ps})[0],400)
   self.assertEqual(x.writes,[]);self.assertEqual(x.executions,[])
 def test_unknown_preset_fields_and_removed_token_setting(self):
  with Harness() as x:
   self.assertEqual(x.management('api/presets',{'expectedVersion':None,'presets':[],'url':'https://bad.invalid'})[0],400)
   self.assertEqual(x.management('api/plan',plan(maxOutputTokens=64))[0],400);self.assertEqual(x.executions,[])

if __name__=='__main__':unittest.main(verbosity=2)
