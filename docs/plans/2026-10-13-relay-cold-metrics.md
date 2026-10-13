# Coturn cold metrics compatibility

The installed relay Agent rejects the owned loopback exporter after Coturn
restarts. The actual HTTP response is 200, 4094 bytes and 73 lines. The exact
deployed parser reaches its final allocation-presence guard: HELP and gauge TYPE
are present, while all seven recognized families have no samples yet. The body,
labels and values were neither reported nor saved.

Accept the empty allocation family only when its valid HELP and exact gauge TYPE
declarations are present. Keep malformed, duplicate, missing and wrong-type
declarations rejected, and retain input bounds and all recognized-label checks.
Existing allocation samples retain their validation and aggregation behavior.

Native allocation counts are diagnostic. PlatformMetrics still obtains the
production allocation count from the broker traffic sample, and real probe
evidence still determines relay eligibility. This change does not alter the
two-second sampling deadline, Unix half-close, framing, authority or keys.

The six focused regression cases initially produced five passes and one real
failure for the legal cold family. After the fix all 13 metrics contract cases
and all 86 Windows relay library cases pass. Linux build, actual cold-response
parsing and post-update relay eligibility remain required before claiming the
cloud blockage is resolved.

Coturn's registration and allocation update behavior is visible in its primary
[exporter source](https://github.com/coturn/coturn/blob/4.6.3/src/apps/relay/prom_server.c).
The installed binary's historical source tag has not been established.
