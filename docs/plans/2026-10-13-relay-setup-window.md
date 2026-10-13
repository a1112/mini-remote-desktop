# Healthy relay setup window

The actual paired self-connection received generation zero with only 13.272
seconds of total validity. Its directory expired at the same instant as the
native connectivity timeout. Public HTTP timestamps show only 9–10 seconds
remaining after the first browser access response, and about 5 seconds on the
next attempt. The authority itself still had several minutes remaining.

Healthy heartbeat leases previously lasted 15 seconds, while the default signed
directory permits 30 seconds. Issue-time coherence correctly clamps the
directory, reservations and TURN credentials to the node lease. A normal
five-second heartbeat age therefore shortened the intended setup window.

Use a shared 35-second healthy lease in the API Registry and Repository, and
the same duration in the Rust relay control model. Keep the directory limit at
30 seconds, all signed-expiry checks, authority and scope bounds, generation
binding, and the candidate fingerprint manifests. No migration or credential
refresh shortcut is added.

The API still requires real process/listener/probe health, retains three
consecutive healthy samples for recovery, and marks failed health unavailable
immediately. A suddenly silent node can now remain selectable up to 35 seconds
instead of 15 seconds. The existing Rust degraded-state selection policy is
unchanged.

Two real heartbeat-to-directory regressions fail with the old lease and pass
with the shared duration. The directory suite passes 57 cases and the
Repository/node API suite passes 130. Rust regressions cover the remaining setup
window, exact expiry rejection and three-sample recovery.

Runtime deployment and a new actual browser-to-local connection are required
before claiming the setup blockage is resolved.
