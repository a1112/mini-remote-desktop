import pathlib,os,subprocess,json,time,importlib.util,sys
from fixture_paths import fixture_root
r=fixture_root();spec=importlib.util.spec_from_file_location('fixture',pathlib.Path(__file__).with_name('turn_lifecycle_fixture.py'));f=importlib.util.module_from_spec(spec);spec.loader.exec_module(f)
base=(r/'fixture-patched/turnserver.conf').read_text().splitlines();exe=r/'build-patched/bin/turnserver';results=[]
for case,flags,expected in [('udp-only',['no-tcp','no-tls'],{'UDP':0.0}),('tcp-only',['no-udp','no-tls'],{'TCP':0.0}),('dtls-enabled',['dtls'],{'UDP':0.0,'TCP':0.0,'TLS/TCP':0.0,'DTLS':0.0})]:
 d=r/('fixture-matrix-'+case);d.mkdir(mode=0o700);ports={'turn':f.free_port(),'tls':f.free_port(),'metrics':f.free_port()};assert len(set(ports.values()))==3;low,high=f.relay_range()
 keys=('listening-port=','tls-listening-port=','prometheus-port=','pidfile=','userdb=','min-port=','max-port=','log-file=');lines=[l for l in base if not l.startswith(keys) and l!='no-dtls']
 lines+=flags+['listening-port='+str(ports['turn']),'tls-listening-port='+str(ports['tls']),'prometheus-port='+str(ports['metrics']),'pidfile='+str(d/'fixture.pid'),'userdb='+str(d/'fixture.sqlite'),'min-port='+str(low),'max-port='+str(high),'log-file='+str(d/'turnserver.log')];f.private_write(d/'turnserver.conf','\n'.join(lines)+'\n')
 result={'case':case,'ports':ports,'expected':expected}
 with open(d/'process.log','wb') as log:
  os.chmod(log.name,0o600);p=subprocess.Popen([str(exe),'-c',str(d/'turnserver.conf')],cwd=d,stdin=subprocess.DEVNULL,stdout=log,stderr=log,start_new_session=True)
  try:
   deadline=time.monotonic()+15
   while True:
    assert p.poll() is None
    try:text,s=f.scrape(ports['metrics']);break
    except (OSError,f.urllib.error.URLError):
     assert time.monotonic()<deadline;time.sleep(.1)
   result['samples']=s;assert s==expected;f.private_write(d/'cold-metrics.txt',text);result['success']=True
  except Exception as e:result.update({'success':False,'exception_class':type(e).__name__})
  finally:
   if p.poll() is None:p.terminate()
   try:result['isolated_process_exit']=p.wait(timeout=8)
   except subprocess.TimeoutExpired:p.kill();result['isolated_process_exit']=p.wait(timeout=3);result['forced_cleanup']=True
 results.append(result)
out={'results':results,'success':all(x['success'] for x in results),'production_ports_used':False};f.private_write(r/'enabled-transport-matrix-result.json',json.dumps(out,indent=2));print(json.dumps(out))
sys.exit(0 if out['success'] else 1)
