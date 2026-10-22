-- Frozen real sealed v2 store, generated with base 5d4ac814 and test protector [85; 32].
-- These encrypted keys belong only to this test fixture, never a real device.
CREATE TABLE audit_events (
           sequence INTEGER PRIMARY KEY,
           timestamp_ms INTEGER NOT NULL,
           action TEXT NOT NULL,
           outcome TEXT NOT NULL,
           session_id TEXT,
           actor_device_id TEXT,
           peer_device_id TEXT,
           transport_kind TEXT,
           reason_code TEXT,
           details_json TEXT NOT NULL,
           previous_hash BLOB NOT NULL,
           event_hash BLOB NOT NULL
         );
INSERT INTO audit_events(sequence,timestamp_ms,action,outcome,session_id,actor_device_id,peer_device_id,transport_kind,reason_code,details_json,previous_hash,event_hash) VALUES (1,1,'trust.approved','allowed',NULL,'fixture-local','d1372c1b3d5a6390c3aeb4be32ac86b76ed7ede7bd8db290e15059a27c0fcf6b',NULL,NULL,'{}',X'',X'7D10ACE31C0D3BE6D6240F571F0A67D443AAA79BBA78965E2E61061FD99895B1');
INSERT INTO audit_events(sequence,timestamp_ms,action,outcome,session_id,actor_device_id,peer_device_id,transport_kind,reason_code,details_json,previous_hash,event_hash) VALUES (2,1,'trust.approved','allowed',NULL,'fixture-local','d5f0faa45d69dce14595272a2d8c33595d75095478709353796bfbc95c94c618',NULL,NULL,'{}',X'7D10ACE31C0D3BE6D6240F571F0A67D443AAA79BBA78965E2E61061FD99895B1',X'F925D0178091266AF89E55F12CE339585950E83AB10F6131219E869CB9176C23');
CREATE TABLE audit_head (
           singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
           next_sequence INTEGER NOT NULL CHECK (next_sequence > 0),
           head_hash BLOB NOT NULL,
           head_seal BLOB NOT NULL
         );
INSERT INTO audit_head(singleton,next_sequence,head_hash,head_seal) VALUES (1,3,X'F925D0178091266AF89E55F12CE339585950E83AB10F6131219E869CB9176C23',X'763BB7699F07E9C38C3E075DF93D72DAF8F3C075C625711982592A9426899371');
CREATE TABLE machine_identity (
           singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
           key_id TEXT NOT NULL UNIQUE,
           epoch INTEGER NOT NULL CHECK (epoch > 0),
           public_key BLOB NOT NULL UNIQUE,
           protected_pkcs8 BLOB NOT NULL,
           created_at_ms INTEGER NOT NULL
         );
INSERT INTO machine_identity(singleton,key_id,epoch,public_key,protected_pkcs8,created_at_ms) VALUES (1,'00abcd2971697a802e8e93b85b3695eb26ac1e768500ba2a42c50ec794f5bb2e',1,X'F053B806C5549FEAA7BA17FC0C48534359CC300FEC1A0DC3F8968B646F0B1710',X'4D52445345414C318B8557717E79228E7FDEB45A7CECA54631008C035ED91DD7E88E0C549BB39690540DB7F27E17CB746A091C37AFCB1938E5FDE4E7015BEEFE10801373ED2BACD7BC6BDDD2AAD3B5956DE62A5C3D14979548AB3BDFD3FFE308C221BA4BF4ABF04676D3942D0A0D27E3562456FD3122F3',1791359759695);
CREATE TABLE schema_migrations (
           version INTEGER PRIMARY KEY
         );
INSERT INTO schema_migrations(version) VALUES (2);
CREATE TABLE store_meta (
           singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
           format_version INTEGER NOT NULL CHECK (format_version = 2),
           store_id BLOB NOT NULL CHECK (length(store_id) = 16),
           generation INTEGER NOT NULL CHECK (generation > 0),
           schema_commitment BLOB NOT NULL CHECK (length(schema_commitment) = 32),
           identity_initialized INTEGER NOT NULL CHECK (identity_initialized IN (0, 1)),
           identity_commitment BLOB NOT NULL CHECK (length(identity_commitment) = 32),
           trust_count INTEGER NOT NULL CHECK (trust_count >= 0),
           trust_commitment BLOB NOT NULL CHECK (length(trust_commitment) = 32),
           audit_initialized INTEGER NOT NULL CHECK (audit_initialized = 1),
           audit_commitment BLOB NOT NULL CHECK (length(audit_commitment) = 32),
           manifest_seal BLOB NOT NULL CHECK (length(manifest_seal) = 32)
         );
INSERT INTO store_meta(singleton,format_version,store_id,generation,schema_commitment,identity_initialized,identity_commitment,trust_count,trust_commitment,audit_initialized,audit_commitment,manifest_seal) VALUES (1,2,X'35219700599D26EA0FB5648FCB2C0AFD',4,X'DB514FE230B2EA14A6D0ECE7BF27FCEF3B7E7BC0DCFDDA083C9E21EFC05E1DC8',1,X'3E625CE3B71D89273AFCC94EAB77DBE672D809012118B09772217D4D48AD14D9',2,X'DA1A6C55CFCF7AFC03A427063CB804F04A04B76AAF29CF91DE82CE63E18DABA9',1,X'46CD9FF3B193BEE980310C287610681F5ECFA6443163AE8D56178DD75921EFFC',X'3706588F2396F2219A5891014EFFB1420E1D57A5ABCE8895DD642139283B652F');
CREATE TABLE store_secrets (
           name TEXT PRIMARY KEY,
           protected_blob BLOB NOT NULL
         );
INSERT INTO store_secrets(name,protected_blob) VALUES ('store_integrity_key_v1',X'4D52445345414C31EF47177CD17F101D92ADE003999DE9BEDD0D22AF306D3D4ECC6C48BDE87A5C692446CDCFBA4C9B97BA57FF114DADF3C50425D210538DD6BC53C48DFE');
INSERT INTO store_secrets(name,protected_blob) VALUES ('audit_hmac_key_v1',X'4D52445345414C3184E2C3AB72CEE7A781F6F1576D1A8F70E9E2926BC7797A9BAE8F1A66B8DBDE6FF4B13D4F78CC839B4A15DEC9778F0150B195B7B3803F85EB59F8CDB2');
CREATE TABLE trusted_devices (
           peer_key_id TEXT PRIMARY KEY,
           public_key BLOB NOT NULL UNIQUE,
           epoch INTEGER NOT NULL CHECK (epoch > 0),
           state TEXT NOT NULL CHECK (state IN ('trusted', 'suspended', 'revoked')),
           revision INTEGER NOT NULL CHECK (revision > 0),
           updated_at INTEGER NOT NULL
         );
INSERT INTO trusted_devices(peer_key_id,public_key,epoch,state,revision,updated_at) VALUES ('d1372c1b3d5a6390c3aeb4be32ac86b76ed7ede7bd8db290e15059a27c0fcf6b',X'7C70B72E369FE04C9E6FC8072BE18E0B6656ACE56217BBABA89C5A97698DA8FD',1,'trusted',1,1791359759696);
INSERT INTO trusted_devices(peer_key_id,public_key,epoch,state,revision,updated_at) VALUES ('d5f0faa45d69dce14595272a2d8c33595d75095478709353796bfbc95c94c618',X'D30D6CD2B0EE0D78A565D7B6567EF954B892DB69E212EA242C0ECD5EFCC4AE53',1,'revoked',1,1791359759698);
PRAGMA user_version = 2;
