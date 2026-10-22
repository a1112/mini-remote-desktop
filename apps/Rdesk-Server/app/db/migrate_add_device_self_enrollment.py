"""Versioned PostgreSQL migration; no existing Device columns are changed."""
from __future__ import annotations

import hashlib

from sqlalchemy import DateTime, String, inspect, text
from sqlalchemy.ext.asyncio import AsyncConnection

from app.db.migrate_add_relay_access import _normalize_check_expression, _table


class DeviceSelfEnrollmentMigrationError(RuntimeError):
    pass


_MAPPING = "device_machine_identities"
_CHALLENGES = "device_self_enrollment_challenges"
SELF_ENROLLMENT_TABLES = frozenset({_MAPPING, _CHALLENGES})


def verify_schema(connection: object, schema: str) -> None:
    inspector = inspect(connection)
    definitions = {
        _MAPPING: {
            "key_id": (String, 64, False), "public_key": (String, 64, False),
            "device_row_id": (String, 36, False), "created_at": (DateTime, None, False),
        },
        _CHALLENGES: {
            "challenge_id": (String, 32, False), "nonce_digest": (String, 64, False),
            "key_id": (String, 64, False), "public_key": (String, 64, False),
            "api_url": (String, 2048, False), "peer_digest": (String, 64, False),
            "issued_at": (DateTime, None, False), "expires_at": (DateTime, None, False),
            "consumed_at": (DateTime, None, True),
        },
    }
    for table, columns in definitions.items():
        actual = {column["name"]: column for column in inspector.get_columns(table, schema=schema)}
        if set(actual) != set(columns):
            raise DeviceSelfEnrollmentMigrationError("self enrollment columns differ")
        for name, (kind, length, nullable) in columns.items():
            column = actual[name]
            if (not isinstance(column["type"], kind) or column["nullable"] != nullable
                    or column["default"] is not None
                    or (length is not None and column["type"].length != length)
                    or (kind is DateTime and not column["type"].timezone)):
                raise DeviceSelfEnrollmentMigrationError("self enrollment column type differs")
        primary = inspector.get_pk_constraint(table, schema=schema)
        if primary["constrained_columns"] != (["key_id"] if table == _MAPPING else ["challenge_id"]):
            raise DeviceSelfEnrollmentMigrationError("self enrollment primary key differs")
    unique = inspector.get_unique_constraints(_MAPPING, schema=schema)
    if len(unique) != 1 or unique[0]["column_names"] != ["device_row_id"]:
        raise DeviceSelfEnrollmentMigrationError("machine mapping uniqueness differs")
    foreign = inspector.get_foreign_keys(_MAPPING, schema=schema)
    if (len(foreign) != 1 or foreign[0]["constrained_columns"] != ["device_row_id"]
            or foreign[0]["referred_columns"] != ["id"]
            or foreign[0]["referred_table"] != "devices"
            or foreign[0]["referred_schema"] not in (None, schema)
            or foreign[0].get("options", {}).get("ondelete") != "RESTRICT"):
        raise DeviceSelfEnrollmentMigrationError("machine mapping foreign key differs")
    if inspector.get_foreign_keys(_CHALLENGES, schema=schema):
        raise DeviceSelfEnrollmentMigrationError("challenge foreign keys differ")
    checks = {
        _MAPPING: {
            "ck_device_machine_key_id": "length(key_id) = 64",
            "ck_device_machine_public_key": "length(public_key) = 64",
        },
        _CHALLENGES: {
            "ck_device_self_challenge_id": "length(challenge_id) = 32",
            "ck_device_self_nonce_digest": "length(nonce_digest) = 64",
            "ck_device_self_key_id": "length(key_id) = 64",
            "ck_device_self_public_key": "length(public_key) = 64",
            "ck_device_self_peer_digest": "length(peer_digest) = 64",
            "ck_device_self_expiry": "expires_at > issued_at",
        },
    }
    for table, expected in checks.items():
        actual = {check["name"]: _normalize_check_expression(check["sqltext"]).replace("(", "").replace(")", "")
                  for check in inspector.get_check_constraints(table, schema=schema)}
        normalized = {name: _normalize_check_expression(expression).replace("(", "").replace(")", "")
                      for name, expression in expected.items()}
        if actual != normalized:
            raise DeviceSelfEnrollmentMigrationError("self enrollment checks differ")
    indexes = {index["name"]: (tuple(index["column_names"]), index["unique"])
               for index in inspector.get_indexes(_CHALLENGES, schema=schema)}
    if indexes != {
        "ix_device_self_issued_at": (("issued_at",), False),
        "ix_device_self_peer_issued": (("peer_digest", "issued_at"), False),
        "ix_device_self_key_issued": (("key_id", "issued_at"), False),
        "ix_device_self_expiry": (("expires_at",), False),
    }:
        raise DeviceSelfEnrollmentMigrationError("self enrollment indexes differ")


async def migrate(connection: AsyncConnection, *, schema: str | None = None) -> None:
    if connection.dialect.name != "postgresql":
        raise DeviceSelfEnrollmentMigrationError("self enrollment migration requires PostgreSQL")
    effective_schema = schema or await connection.scalar(text("SELECT current_schema()"))
    mapping, challenges, devices, versions = (
        _table(effective_schema, name) for name in
        (_MAPPING, _CHALLENGES, "devices", "device_self_enrollment_schema_migrations")
    )
    lock = int.from_bytes(hashlib.sha256(
        b"MRD_DEVICE_SELF_ENROLLMENT_MIGRATION_V1\0" + effective_schema.encode()
    ).digest()[:8], "big", signed=True)
    await connection.execute(text("SELECT pg_advisory_xact_lock(:key)"), {"key": lock})
    await connection.execute(text(f"CREATE TABLE IF NOT EXISTS {versions} (version INTEGER PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())"))
    applied = set((await connection.execute(text(f"SELECT version FROM {versions}"))).scalars())
    if applied not in (set(), {1}):
        raise DeviceSelfEnrollmentMigrationError("self enrollment migration versions differ")
    if not applied:
        existing = await connection.run_sync(lambda sync: any(
            inspect(sync).has_table(name, schema=effective_schema) for name in SELF_ENROLLMENT_TABLES
        ))
        if existing:
            raise DeviceSelfEnrollmentMigrationError("self enrollment tables exist without migration")
        await connection.execute(text(f"""
            CREATE TABLE {mapping} (
                key_id VARCHAR(64) PRIMARY KEY,
                public_key VARCHAR(64) NOT NULL,
                device_row_id VARCHAR(36) NOT NULL UNIQUE REFERENCES {devices}(id) ON DELETE RESTRICT,
                created_at TIMESTAMPTZ NOT NULL,
                CONSTRAINT ck_device_machine_key_id CHECK (length(key_id) = 64),
                CONSTRAINT ck_device_machine_public_key CHECK (length(public_key) = 64)
            )
        """))
        await connection.execute(text(f"""
            CREATE TABLE {challenges} (
                challenge_id VARCHAR(32) PRIMARY KEY,
                nonce_digest VARCHAR(64) NOT NULL,
                key_id VARCHAR(64) NOT NULL,
                public_key VARCHAR(64) NOT NULL,
                api_url VARCHAR(2048) NOT NULL,
                peer_digest VARCHAR(64) NOT NULL,
                issued_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                consumed_at TIMESTAMPTZ,
                CONSTRAINT ck_device_self_challenge_id CHECK (length(challenge_id) = 32),
                CONSTRAINT ck_device_self_nonce_digest CHECK (length(nonce_digest) = 64),
                CONSTRAINT ck_device_self_key_id CHECK (length(key_id) = 64),
                CONSTRAINT ck_device_self_public_key CHECK (length(public_key) = 64),
                CONSTRAINT ck_device_self_peer_digest CHECK (length(peer_digest) = 64),
                CONSTRAINT ck_device_self_expiry CHECK (expires_at > issued_at)
            )
        """))
        for name, columns in (
            ("ix_device_self_issued_at", "issued_at"),
            ("ix_device_self_peer_issued", "peer_digest, issued_at"),
            ("ix_device_self_key_issued", "key_id, issued_at"),
            ("ix_device_self_expiry", "expires_at"),
        ):
            await connection.execute(text(f"CREATE INDEX {name} ON {challenges} ({columns})"))
        await connection.run_sync(lambda sync: verify_schema(sync, effective_schema))
        await connection.execute(text(f"INSERT INTO {versions} (version) VALUES (1)"))
    await connection.run_sync(lambda sync: verify_schema(sync, effective_schema))
    unvalidated = await connection.scalar(text("""
        SELECT count(*) FROM pg_constraint
        WHERE conrelid IN (CAST(:mapping AS regclass), CAST(:challenges AS regclass))
          AND contype = 'c' AND NOT convalidated
    """), {"mapping": mapping, "challenges": challenges})
    if unvalidated:
        raise DeviceSelfEnrollmentMigrationError("self enrollment checks are unvalidated")
