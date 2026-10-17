import os
"""真实插件子进程的 SDK 握手、受管回调、写入范围和生命周期测试，不访问真实服务器"""
import base64,zlib,pathlib,json,struct,subprocess,threading,queue,copy,unittest,time
ROOT=pathlib.Path(__file__).resolve().parent.parent;BIN=pathlib.Path(os.environ.get('CPR_TEST_PLUGIN_BIN',str(ROOT/'backend/target/debug'/('cpr-account-dialogue.exe' if os.name=='nt' else 'cpr-account-dialogue'))))
MANIFEST=json.loads(subprocess.check_output([str(BIN),'--print-manifest']))
POLICY={'mode':'denylist','models':['claude-one']}
ACCOUNT={'id':'acct_test_gpt','name':'GPT','provider':'openai','enabled':True,'status':'normal','modelAccess':copy.deepcopy(POLICY),'apiKey':'DO_NOT_EXPOSE_SENTINEL','concurrencyLimit':3,'weight':1}
class Harness:
 def __init__(self,doc=None,version=None,fail_put=False,fail_native=False):
  self.proc=subprocess.Popen([str(BIN)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  self.frames=queue.Queue();self.read_error=None;self.seq=99;self.accounts=[copy.deepcopy(ACCOUNT)];self.doc=doc;self.version=version;self.fail_put=fail_put;self.fail_native=fail_native;self.callbacks=[];self.http_requests=[];self.writes=[];self.bodies={};self.body_seq=0
  threading.Thread(target=self.reader,daemon=True).start()
  self.send({'type':'hello','handshake':{'protocol_version':4,'artifact_sha256':'a'*64,'plugin_id':'donzy.account-dialogue','instance_id':'test-instance','generation':1,'incarnation':'test-incarnation','configuration':{},'contributes':MANIFEST['contributes']}})
  ready,payload=self.read();assert ready['type']=='ready' and ready['protocol_version']==4 and not payload
  self.call('plugin.register',{},b'','registration')
 def reader(self):
  try:
   def exact(n):
    data=b''
    while len(data)<n:
     part=self.proc.stdout.read(n-len(data))
     if not part:raise EOFError()
     data+=part
    return data
   while True:
    head=exact(12);n,b=struct.unpack('>IQ',head);meta=json.loads(exact(n));payload=exact(b)
    request_bytes=meta.pop('request_context_bytes',0);data_bytes=meta.pop('data_bytes',0)
    if request_bytes:meta['context']['request']=json.loads(payload[:request_bytes])
    data=payload[request_bytes:request_bytes+data_bytes];payload=payload[request_bytes+data_bytes:]
    if data_bytes:
     value=json.loads(data)
     if meta['type']=='hello':meta['handshake']['configuration']=value
     else:meta['result' if meta['type']=='result' else 'params']=value
    self.frames.put((meta,payload))
  except Exception as e:self.frames.put(e)
 def send(self,meta,payload=b''):
  meta=copy.deepcopy(meta);business=b''
  if meta['type']=='hello':business=json.dumps(meta['handshake'].pop('configuration'),separators=(',',':')).encode()
  elif meta['type'] in ['call','callback','result']:business=json.dumps(meta.pop('result' if meta['type']=='result' else 'params'),separators=(',',':'),ensure_ascii=False).encode()
  if business:meta['data_bytes']=len(business)
  data=json.dumps(meta,separators=(',',':'),ensure_ascii=False).encode();assert len(data)<=65536;self.proc.stdin.write(struct.pack('>IQ',len(data),len(business)+len(payload))+data+business+payload);self.proc.stdin.flush()
 def read(self):
  f=self.frames.get(timeout=5)
  if isinstance(f,Exception):raise RuntimeError('Plugin exited unexpectedly: '+self.proc.stderr.read().decode(errors='replace')) from f
  return f
 def callback(self,meta,payload):
  method=meta['method'];params=meta['params'];self.callbacks.append(method);result=None;body=b'';error=None
  if method=='host.state.get':
   result={'record':None if self.doc is None else {'value':{'encoding':'zlib-base64','data':base64.b64encode(zlib.compress(json.dumps(self.doc).encode())).decode()},'version':self.version,'schema_version':1}}
  elif method=='host.state.put':
   if self.fail_put:error={'code':'conflict','message':'test conflict','send_state':'not_sent'}
   else:
    assert params['expected_version']==self.version
    self.doc=json.loads(zlib.decompress(base64.b64decode(params['value']['data'])));self.version=(self.version or 0)+1;result={'version':self.version}
  elif method=='host.http.dispatch':
   uri=params['uri'];verb=params['method'];self.http_requests.append((verb,uri));headers=params['headers'];assert all(h['name'] not in ['authorization','cookie'] for h in headers)
   assert uri.startswith('/api/admin/accounts')
   code=200;status=200;data=None
   if verb=='GET' and uri.startswith('/api/admin/accounts?'):data={'items':copy.deepcopy(self.accounts),'page':{},'summary':{}}
   elif verb=='POST' and uri=='/api/admin/accounts/batch-update':
    patch=json.loads(payload);self.writes.append(patch);assert set(patch)=={'accountIds','modelAccess'}
    if self.fail_native:code=1001;status=503
    else:
     for account in self.accounts:
      if account['id'] in patch['accountIds']:account['modelAccess']=copy.deepcopy(patch['modelAccess'])
    data={'updated':1}
   elif uri.startswith('/api/admin/accounts/models?') or uri=='/api/admin/accounts/models/refresh':
    if 'acct_second' in uri or (verb=='POST' and json.loads(payload).get('accountId')=='acct_second'):data={'models':[{'id':'claude-two'},{'id':'custom-image'},{'id':'__not_public'}]}
    elif 'acct_failed' in uri or (verb=='POST' and json.loads(payload).get('accountId')=='acct_failed'):code=50301;status=503
    else:data={'models':[{'id':'claude-one'},{'id':'custom-image'},{'id':'__not_public'}]}
   else:raise AssertionError((verb,uri))
   self.body_seq+=1;handle='body_'+str(self.body_seq);self.bodies[handle]=json.dumps({'code':code,'message':'','data':data}).encode()
   result={'status':status,'version':'HTTP/1.1','headers':[],'body':{'kind':'handle','handle':handle},'response':None,'session':False}
  elif method=='host.http.body_read':
   data=self.bodies[params['handle']]
   if data:self.bodies[params['handle']]=b'';result={'eof':False,'trailers':None};body=data
   else:result={'eof':True,'trailers':None}
  elif method=='host.http.body_close':result={}
  else:raise AssertionError('Unexpected callback '+method)
  if error:self.send({'type':'error','id':meta['id'],'error':error})
  else:self.send({'type':'result','id':meta['id'],'result':result},body)
 def context(self,id,stage):return {'call_id':id,'instance_id':'test-instance','generation':1,'incarnation':'test-incarnation','stage':stage,'timeout_ms':30000,'resource_stream':False,'resource_scope_id':'scope_'+str(id)}
 def call(self,method,params,payload=b'',stage='management'):
  self.seq+=2;id=self.seq;self.send({'type':'call','id':id,'method':method,'context':self.context(id,stage),'params':params},payload)
  while True:
   meta,body=self.read()
   if meta['type']=='callback':self.callback(meta,body)
   elif meta['type']=='credit':continue
   elif meta['type']=='result' and meta['id']==id:return meta['result'],body
   elif meta['type']=='error' and meta['id']==id:raise AssertionError(meta)
   else:raise AssertionError(meta)
 def management(self,path,value=None,method=None):
  method=method or ('GET' if value is None else 'POST');params={'method':method,'path':path,'query':'','content_type':None if value is None else 'application/json','headers':[]}
  result,payload=self.call('management.handle',params,b'' if value is None else json.dumps(value).encode());return result['status'],json.loads(payload)
 def close(self):
  if self.proc.poll() is None:
   self.send({'type':'quiesce'});self.send({'type':'shutdown'});self.proc.stdin.close()
   try:self.proc.wait(timeout=5)
   except subprocess.TimeoutExpired:self.proc.kill();raise AssertionError('Shutdown did not drain')
  assert self.proc.returncode==0,self.proc.stderr.read().decode(errors='replace')
 def __enter__(self):return self
 def __exit__(self,*_):self.close()
