#include "prom.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
static prom_gauge_t *g;
static const char *labels[]={"UDP"};
static void value(double expected){char *b=prom_collector_registry_bridge(PROM_COLLECTOR_REGISTRY_DEFAULT);assert(b);char *s=strstr(b,"turn_total_allocations{type=\"UDP\"} ");assert(s);double v=-1;assert(sscanf(s,"turn_total_allocations{type=\"UDP\"} %lf",&v)==1);assert(v==expected);free(b);}
static void *inc(void *ignored){(void)ignored;for(int i=0;i<10000;i++)assert(prom_gauge_inc(g,labels)==0);return NULL;}
static void *zero(void *ignored){(void)ignored;for(int i=0;i<10000;i++)assert(prom_gauge_add(g,0,labels)==0);return NULL;}
int main(void){
 assert(prom_collector_registry_default_init()==0);
 const char *keys[]={"type"};g=prom_collector_registry_must_register_metric(prom_gauge_new("turn_total_allocations","Current allocations",1,keys));assert(g);
 char *empty=prom_collector_registry_bridge(PROM_COLLECTOR_REGISTRY_DEFAULT);assert(empty && !strstr(empty,"turn_total_allocations{type="));free(empty);
 assert(prom_gauge_add(g,0,labels)==0);value(0);
 assert(prom_gauge_inc(g,labels)==0);value(1);
 assert(prom_gauge_add(g,0,labels)==0);value(1);
 pthread_t threads[4];assert(pthread_create(&threads[0],NULL,inc,NULL)==0);assert(pthread_create(&threads[1],NULL,inc,NULL)==0);assert(pthread_create(&threads[2],NULL,zero,NULL)==0);assert(pthread_create(&threads[3],NULL,zero,NULL)==0);
 for(int i=0;i<4;i++){assert(pthread_join(threads[i],NULL)==0);}
 value(20001);
 assert(prom_gauge_dec(g,labels)==0);value(20000);assert(prom_gauge_add(g,0,labels)==0);value(20000);
 assert(prom_gauge_add(NULL,0,labels)!=0);
 prom_counter_t *c=prom_counter_new("a_counter","Counter",0,NULL);assert(c);assert(prom_gauge_add((prom_gauge_t*)c,0,NULL)!=0);
 puts("{\"success\":true,\"real_prom_c_library\":true,\"descriptor_without_sample_observed\":true,\"cold_add_zero_creates_sample\":true,\"existing_nonzero_preserved\":true,\"concurrent_increments\":20000,\"concurrent_add_zero\":20000,\"final_before_decrement\":20001,\"null_and_wrong_kind_checked\":true}");return 0;
}
