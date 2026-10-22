import pathlib,shutil,hashlib,json,difflib,os
from fixture_paths import fixture_root
r=fixture_root();s=r/'source/coturn-de0c9b28f22281a0251d7e98aa8a097895a5b185';p=r/'source-patched'
pins=json.loads(pathlib.Path(__file__).with_name('SOURCE-PINS.json').read_text())
for rel,digests in pins['changed_files'].items():assert hashlib.sha256((s/rel).read_bytes()).hexdigest()==digests['before']
assert not p.exists();shutil.copytree(s,p)
def replace1(text,a,b):
 assert text.count(a)==1
 return text.replace(a,b,1)
rel='src/apps/relay/prom_server.c';old=(s/rel).read_text();new=old
new=replace1(new,'void start_prometheus_server(void) {\n  if (!turn_params.prometheus) {\n    TURN_LOG_FUNC(TURN_LOG_LEVEL_INFO, "prometheus collector disabled, not started\\n");\n    return;\n  }\n  prom_collector_registry_default_init();', '''void initialize_prometheus_metrics(void) {
  if (!turn_params.prometheus) {
    return;
  }
  if (prom_collector_registry_default_init() != 0) {
    TURN_LOG_FUNC(TURN_LOG_LEVEL_ERROR, "could not initialize prometheus registry\\n");
    exit(EXIT_FAILURE);
  }''')
needle='''  turn_total_allocations = prom_collector_registry_must_register_metric(
      prom_gauge_new("turn_total_allocations", "Represents current allocations number", 1, typeLabel));'''
replacement=needle+'''
  // Materialize only enabled client transports before allocation workers start.
  // Adding zero creates a missing series without resetting an existing count.
  const SOCKET_TYPE types[] = {UDP_SOCKET, TCP_SOCKET, TLS_SOCKET, DTLS_SOCKET};
  const bool enabled[] = {!turn_params.no_udp, !turn_params.no_tcp, !turn_params.no_tls,
#if DTLS_SUPPORTED
                          turn_params.dtls
#else
                          false
#endif
  };
  for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); ++i) {
    if (enabled[i]) {
      const char *values[] = {socket_type_name(types[i])};
      if (prom_gauge_add(turn_total_allocations, 0, values) != 0) {
        TURN_LOG_FUNC(TURN_LOG_LEVEL_ERROR, "could not initialize allocation metric for %s\\n", values[0]);
        exit(EXIT_FAILURE);
      }
    }
  }'''
new=replace1(new,needle,replacement)
new=replace1(new,'  // some flags appeared first in microhttpd v0.9.53','''}

void start_prometheus_server(void) {
  if (!turn_params.prometheus) {
    TURN_LOG_FUNC(TURN_LOG_LEVEL_INFO, "prometheus collector disabled, not started\\n");
    return;
  }

  // some flags appeared first in microhttpd v0.9.53''')
new=replace1(new,'#else\n\nvoid start_prometheus_server(void) {','#else\n\nvoid initialize_prometheus_metrics(void) {}\n\nvoid start_prometheus_server(void) {')
(p/rel).write_text(new)
rel='src/apps/relay/prom_server.h';oldh=(s/rel).read_text();newh=replace1(oldh,'void start_prometheus_server(void);','/* Initialize descriptors and enabled allocation series before setup_server(). */\nvoid initialize_prometheus_metrics(void);\nvoid start_prometheus_server(void);');(p/rel).write_text(newh)
rel='src/apps/relay/mainrelay.c';oldm=(s/rel).read_text();newm=replace1(oldm,'  setup_server();','  initialize_prometheus_metrics();\n  setup_server();');(p/rel).write_text(newm)
files=['src/apps/relay/prom_server.c','src/apps/relay/prom_server.h','src/apps/relay/mainrelay.c'];patch=''.join(''.join(difflib.unified_diff((s/f).read_text().splitlines(keepends=True),(p/f).read_text().splitlines(keepends=True),fromfile='a/'+f,tofile='b/'+f)) for f in files)
bundled=pathlib.Path(__file__).with_name('allocation-cold-start.patch').read_bytes()
assert hashlib.sha256(bundled).hexdigest()==pins['patch_sha256'] and patch.encode()==bundled
for rel,digests in pins['changed_files'].items():assert hashlib.sha256((p/rel).read_bytes()).hexdigest()==digests['after']
(r/'allocation-cold-start.patch').write_text(patch);os.chmod(r/'allocation-cold-start.patch',0o600)
receipt={'patch_sha256':hashlib.sha256(patch.encode()).hexdigest(),'changed_files':{f:{'before':hashlib.sha256((s/f).read_bytes()).hexdigest(),'after':hashlib.sha256((p/f).read_bytes()).hexdigest()} for f in files},'disabled_types_not_materialized':True,'init_before_setup_server':newm.index('  initialize_prometheus_metrics();')<newm.index('  setup_server();'),'http_start_order_unchanged':newm.index('  setup_server();')<newm.index('  drop_privileges();')<newm.index('  start_prometheus_server();')}
(r/'patch-receipt.json').write_text(json.dumps(receipt,indent=2));os.chmod(r/'patch-receipt.json',0o600)
print(json.dumps(receipt));print(patch)
