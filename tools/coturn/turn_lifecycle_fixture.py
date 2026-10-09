#!/usr/bin/env python3
import os,sys,pathlib,socket,struct,secrets,time,json,subprocess,urllib.request,re,hashlib,hmac,base64,ssl,datetime,ipaddress,traceback
from fixture_paths import fixture_root
ROOT=fixture_root()
COOKIE=0x2112A442
FORBIDDEN={22,3478,5349,9443,9530,9641}
def private_write(p,data):
 fd=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
 with os.fdopen(fd,'wb') as f:f.write(data if isinstance(data,bytes) else data.encode());f.flush();os.fsync(f.fileno())
def free_port():
 for _ in range(40):
  with socket.socket() as s:s.bind(('127.0.0.1',0));p=s.getsockname()[1]
  if p in FORBIDDEN:continue
  try:
   with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as u:u.bind(('127.0.0.1',p))
   return p
  except OSError:continue
 raise RuntimeError('no_fixture_port')
def relay_range():
 for _ in range(40):
  low=secrets.randbelow(5000)+40000;held=[]
  try:
   for p in range(low,low+8):
    s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);s.bind(('127.0.0.1',p));held.append(s)
   return low,low+7
  except OSError:pass
  finally:
   for s in held:s.close()
 raise RuntimeError('no_fixture_relay_range')
def attr(t,v):return struct.pack('!HH',t,len(v))+v+b'\0'*((-len(v))%4)
def message(typ,tx,attrs,key=None):
 body=b''.join(attr(t,v) for t,v in attrs)
 header=struct.pack('!HHI12s',typ,len(body)+(24 if key else 0),COOKIE,tx)
 if key:body+=attr(0x0008,hmac.new(key,header+body,hashlib.sha1).digest())
 return header+body
def parse(data,tx,key=None):
 typ,n,cookie,got=struct.unpack('!HHI12s',data[:20]);assert got==tx and cookie==COOKIE and n%4==0 and len(data)==20+n
 a={};pos=20;mi=False
 while pos<len(data):
  t,l=struct.unpack('!HH',data[pos:pos+4]);v=data[pos+4:pos+4+l];assert len(v)==l
  if t==0x0008 and key:
   header=data[:2]+struct.pack('!H',pos-20+24)+data[4:20]
   assert hmac.compare_digest(v,hmac.new(key,header+data[20:pos],hashlib.sha1).digest());mi=True
  a[t]=v;pos+=4+l+((-l)%4)
 assert pos==len(data)
 if key:assert mi
 return typ,a
class Client:
 def __init__(self,kind,port,user,password,realm,cert):
  self.kind=kind;self.user=user;self.password=password;self.realm=realm;self.nonce=None;self.key=hashlib.md5((user+':'+realm+':'+password).encode()).digest()
  self.s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM if kind=='UDP' else socket.SOCK_STREAM);self.s.settimeout(5);self.s.connect(('127.0.0.1',port))
  if kind=='TLS/TCP':self.s=ssl.create_default_context(cafile=str(cert)).wrap_socket(self.s,server_hostname='127.0.0.1')
 def exact(self,n):
  out=b''
  while len(out)<n:
   chunk=self.s.recv(n-len(out));assert chunk;out+=chunk
  return out
 def exchange(self,typ,attrs,auth=True):
  tx=secrets.token_bytes(12)
  if auth:attrs=attrs+[(0x0006,self.user.encode()),(0x0014,self.realm.encode()),(0x0015,self.nonce)]
  packet=message(typ,tx,attrs,self.key if auth else None)
  if self.kind=='UDP':self.s.send(packet);data=self.s.recv(65536)
  else:self.s.sendall(packet);head=self.exact(20);data=head+self.exact(struct.unpack('!H',head[2:4])[0])
  return parse(data,tx,self.key if auth else None)
 def allocate(self):
  requested=[(0x0019,b'\x11\0\0\0'),(0x000d,struct.pack('!I',300))]
  typ,a=self.exchange(0x0003,requested,False);assert typ==0x0113 and a[0x0009][2]*100+a[0x0009][3]==401
  assert a[0x0014].decode()==self.realm;self.nonce=a[0x0015]
  typ,a=self.exchange(0x0003,requested);assert typ==0x0103 and 0x0016 in a
  assert struct.unpack('!I',a[0x000d])[0]>=300
  return {'transport':self.kind,'authenticated_allocate_success':True,'challenge_401':True,'response_integrity_verified':True,'lifetime':struct.unpack('!I',a[0x000d])[0]}
 def refresh(self,lifetime):
  typ,a=self.exchange(0x0004,[(0x000d,struct.pack('!I',lifetime))]);assert typ==0x0104 and struct.unpack('!I',a[0x000d])[0]==lifetime
  return {'transport':self.kind,'authenticated_refresh_success':True,'response_integrity_verified':True,'requested_lifetime':lifetime,'ack_lifetime':struct.unpack('!I',a[0x000d])[0]}
def scrape(port):
 with urllib.request.build_opener(urllib.request.ProxyHandler({})).open('http://127.0.0.1:'+str(port)+'/metrics',timeout=2) as r:assert r.status==200;text=r.read(262144).decode()
 samples={}
 for l in text.splitlines():
  if l.startswith('turn_total_allocations{'):
   m=re.fullmatch(r'turn_total_allocations\{type="([^"\\]+)"\} ([0-9.+eE-]+)',l);assert m and m[1] not in samples;samples[m[1]]=float(m[2])
 return text,samples
def wait_samples(port,expected,proc):
 deadline=time.monotonic()+8
 while time.monotonic()<deadline:
  assert proc.poll() is None
  text,s=scrape(port)
  if s==expected:return text,s
  time.sleep(.05)
 raise AssertionError('allocation_series_did_not_match_expected')
def main():
 assert os.geteuid()==0
 phase=sys.argv[1];assert phase in ('baseline','patched');suffix='-'+sys.argv[2] if len(sys.argv)>2 else '';assert re.fullmatch(r'[-a-z0-9]*',suffix);r=ROOT/('fixture-'+phase+suffix);r.mkdir(mode=0o700)
 ports={'turn':free_port(),'tls':free_port(),'metrics':free_port()};assert len(set(ports.values()))==3
 low,high=relay_range();secret=secrets.token_urlsafe(32);realm='mrd-isolated-'+secrets.token_hex(8)
 user=str(int(time.time())+3600)+':isolated';password=base64.b64encode(hmac.new(secret.encode(),user.encode(),hashlib.sha1).digest()).decode()
 from cryptography import x509
 from cryptography.hazmat.primitives import hashes,serialization
 from cryptography.hazmat.primitives.asymmetric import ec
 from cryptography.x509.oid import NameOID
 key=ec.generate_private_key(ec.SECP256R1());now=datetime.datetime.now(datetime.timezone.utc);name=x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'mrd-isolated-fixture')])
 cert=x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key()).serial_number(x509.random_serial_number()).not_valid_before(now-datetime.timedelta(minutes=1)).not_valid_after(now+datetime.timedelta(hours=1)).add_extension(x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]),critical=False).sign(key,hashes.SHA256())
 private_write(r/'cert.pem',cert.public_bytes(serialization.Encoding.PEM));private_write(r/'key.pem',key.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.PKCS8,serialization.NoEncryption()))
 config='\n'.join(['listening-ip=127.0.0.1','relay-ip=127.0.0.1','listening-port='+str(ports['turn']),'tls-listening-port='+str(ports['tls']),'relay-threads=2','no-dtls','no-cli','no-rfc5780','fingerprint','use-auth-secret','static-auth-secret='+secret,'realm='+realm,'no-multicast-peers','allow-loopback-peers','no-tcp-relay','prometheus','prometheus-address=127.0.0.1','prometheus-port='+str(ports['metrics']),'cert='+str(r/'cert.pem'),'pkey='+str(r/'key.pem'),'pidfile='+str(r/'fixture.pid'),'userdb='+str(r/'fixture.sqlite'),'min-port='+str(low),'max-port='+str(high),'log-file='+str(r/'turnserver.log'),'simple-log','no-stdout-log'])+'\n'
 private_write(r/'turnserver.conf',config)
 exe=ROOT/('build-'+('baseline' if phase=='baseline' else 'patched'))/'bin/turnserver';clients=[];result={'phase':phase,'ports':ports,'relay_range':[low,high],'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'production_ports_used':False,'own_credentials_only':True,'events':[]}
 with open(r/'process.log','wb') as log:
  os.chmod(log.name,0o600);proc=subprocess.Popen([str(exe),'-c',str(r/'turnserver.conf')],stdin=subprocess.DEVNULL,stdout=log,stderr=log,start_new_session=True,cwd=r)
  result['pid']=proc.pid
  try:
   deadline=time.monotonic()+15
   while True:
    assert proc.poll() is None
    try:text,cold=scrape(ports['metrics']);break
    except (OSError,urllib.error.URLError):
     if time.monotonic()>=deadline:raise
     time.sleep(.1)
   private_write(r/'cold-metrics.txt',text);result['cold_samples']=cold
   if phase=='baseline':
    result['expected_cold_types']=['UDP','TCP','TLS/TCP'];result['red_failure_observed']=cold=={};assert result['red_failure_observed'];result['success']=True
   else:
    zero={'UDP':0.0,'TCP':0.0,'TLS/TCP':0.0};assert cold==zero;expected=zero.copy()
    for kind in ['UDP','TCP','TLS/TCP']:
     client=Client(kind,ports['tls'] if kind=='TLS/TCP' else ports['turn'],user,password,realm,r/'cert.pem');clients.append(client);event=client.allocate();expected[kind]+=1;_,samples=wait_samples(ports['metrics'],expected,proc);event['samples']=samples;result['events'].append(event)
     event=client.refresh(600);_,samples=wait_samples(ports['metrics'],expected,proc);event['samples']=samples;event['ordinary_refresh_did_not_increment']=True;result['events'].append(event)
    private_write(r/'active-metrics.txt',scrape(ports['metrics'])[0])
    for client in clients:
     event=client.refresh(0);expected[client.kind]-=1;_,samples=wait_samples(ports['metrics'],expected,proc);event['samples']=samples;event['zero_verified_before_client_close']=True;result['events'].append(event)
    private_write(r/'released-metrics.txt',scrape(ports['metrics'])[0]);result['final_samples']=samples;assert samples==zero;result['success']=True
  except Exception as e:
   result['success']=False;result['exception_class']=type(e).__name__;result['traceback_frames']=[{'function':f.name,'line':f.lineno} for f in traceback.extract_tb(e.__traceback__)]
  finally:
   for c in clients:c.s.close()
   if proc.poll() is None:proc.terminate()
   try:result['isolated_process_exit']=proc.wait(timeout=8)
   except subprocess.TimeoutExpired:proc.kill();result['isolated_process_exit']=proc.wait(timeout=3);result['forced_isolated_cleanup']=True
 private_write(ROOT/(phase+suffix+'-fixture-result.json'),json.dumps(result,indent=2));print(json.dumps(result));sys.exit(0 if result.get('success') else 1)
if __name__=='__main__':main()
