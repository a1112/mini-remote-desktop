"""Upgrade only the known account authority; never accept nullable user drift."""

import re
from sqlalchemy import (
    BigInteger,
    CheckConstraint,
    DateTime,
    MetaData,
    String,
    inspect,
    select,
    text,
)
from sqlalchemy.schema import CreateTable
from app.db.session import Base
from app.models.browser_controller import BrowserController
from app.models.session_request import SessionRequest
from app.models.device_temporary_access import DeviceTemporaryAccess, GuestAccessAttempt

GUEST_TABLES = {"device_temporary_access", "guest_access_attempts"}
SESSION_GUEST_COLUMNS = {
    "authority_kind",
    "temporary_access_generation",
    "target_auth_version",
    "authority_expires_at",
}
BROWSER_GUEST_COLUMNS = SESSION_GUEST_COLUMNS - {"authority_expires_at"}


def authority_expression(expression):
    """Compare parsed boolean structure, preserving AND/OR grouping and NULL."""
    from app.db.migrate_add_relay_access import _CHECK_CAST

    raw = _CHECK_CAST.sub("", str(expression).lower()).replace('"', "")
    pattern = re.compile(
        r"\s*(>=|<=|<>|!=|=|>|<|\(|\)|'[^']*'|[a-z_][a-z_0-9]*|[0-9]+)"
    )
    tokens = []
    offset = 0
    while offset < len(raw):
        match = pattern.match(raw, offset)
        if match is None:
            if raw[offset:].strip():
                raise RuntimeError("guest authority check differs")
            break
        tokens.append(match[1])
        offset = match.end()
    if len(tokens) > 2048:
        raise RuntimeError("guest authority check differs")
    position = 0

    def scalar():
        nonlocal position
        if position >= len(tokens):
            raise RuntimeError("guest authority check differs")
        token = tokens[position]
        position += 1
        if token == "(":
            value = scalar()
            if position >= len(tokens) or tokens[position] != ")":
                raise RuntimeError("guest authority check differs")
            position += 1
            return value
        if token == "length" and position < len(tokens) and tokens[position] == "(":
            position += 1
            value = scalar()
            if position >= len(tokens) or tokens[position] != ")":
                raise RuntimeError("guest authority check differs")
            position += 1
            return ("length", value)
        if token in {
            "and",
            "or",
            "is",
            "not",
            ")",
            ">=",
            "<=",
            "<>",
            "!=",
            "=",
            ">",
            "<",
        }:
            raise RuntimeError("guest authority check differs")
        return token

    def factor():
        nonlocal position
        if position >= len(tokens):
            raise RuntimeError("guest authority check differs")
        if tokens[position] == "(":
            saved = position
            try:
                position += 1
                value = disjunction()
                if position >= len(tokens) or tokens[position] != ")":
                    raise RuntimeError("guest authority check differs")
                position += 1
                return value
            except RuntimeError:
                position = saved
        left = scalar()
        if position >= len(tokens):
            raise RuntimeError("guest authority check differs")
        operator = tokens[position]
        position += 1
        if operator == "is":
            if position < len(tokens) and tokens[position] == "not":
                operator += " not"
                position += 1
            if position >= len(tokens) or tokens[position] != "null":
                raise RuntimeError("guest authority check differs")
            position += 1
            return (operator, left, "null")
        if operator not in {"=", ">=", ">", "<=", "<", "<>", "!="}:
            raise RuntimeError("guest authority check differs")
        return (operator, left, scalar())

    def associative(operator, values):
        flattened = []
        for value in values:
            if isinstance(value, tuple) and value[0] == operator:
                flattened.extend(value[1])
            else:
                flattened.append(value)
        return flattened[0] if len(flattened) == 1 else (operator, tuple(flattened))

    def conjunction():
        nonlocal position
        values = [factor()]
        while position < len(tokens) and tokens[position] == "and":
            position += 1
            values.append(factor())
        return associative("and", values)

    def disjunction():
        nonlocal position
        values = [conjunction()]
        while position < len(tokens) and tokens[position] == "or":
            position += 1
            values.append(conjunction())
        return associative("or", values)

    result = disjunction()
    if position != len(tokens):
        raise RuntimeError("guest authority check differs")
    return result


def expected_authority_check(table):
    return next(
        str(c.sqltext)
        for c in table.constraints
        if isinstance(c, CheckConstraint) and c.name.endswith("_authority")
    )


def verify_guest_session_authority(connection, schema=None, *, inspector=None):
    inspector = inspector or inspect(connection)
    columns = {
        c["name"]: c for c in inspector.get_columns("session_requests", schema=schema)
    }
    if not SESSION_GUEST_COLUMNS <= columns.keys():
        raise RuntimeError("guest session authority schema differs")
    expected = {
        "authority_kind": (String, 24, False, "'account'"),
        "temporary_access_generation": (BigInteger, None, True, ""),
        "target_auth_version": (BigInteger, None, True, ""),
        "authority_expires_at": (DateTime, None, True, ""),
        "requester_user_id": (String, 36, True, ""),
    }
    from app.db.migrate_add_relay_access import _normalize_server_default

    for name, (kind, length, nullable, default) in expected.items():
        column = columns.get(name)
        if (
            column is None
            or column["nullable"] is not nullable
            or not isinstance(column["type"], kind)
            or (length is not None and column["type"].length != length)
            or (
                connection.dialect.name == "postgresql"
                and kind is DateTime
                and not column["type"].timezone
            )
            or _normalize_server_default(column.get("default")) != default
            or column.get("computed")
            or column.get("identity")
        ):
            raise RuntimeError("guest session authority schema differs: " + name)
    check = next(
        (
            c
            for c in inspector.get_check_constraints("session_requests", schema=schema)
            if c["name"] == "ck_session_requests_authority"
        ),
        None,
    )
    if (
        check is None
        or (check.get("dialect_options") or {}).get("postgresql_not_valid")
        or authority_expression(check["sqltext"])
        != authority_expression(expected_authority_check(SessionRequest.__table__))
    ):
        raise RuntimeError("guest session authority check differs")
    if connection.dialect.name == "postgresql":
        current_schema = schema or inspector.default_schema_name
        state = connection.execute(
            text(
                "SELECT convalidated,condeferrable,condeferred FROM pg_constraint WHERE conrelid=to_regclass(:table) AND conname=:name"
            ),
            {
                "table": current_schema + ".session_requests",
                "name": "ck_session_requests_authority",
            },
        ).one_or_none()
        if state is None or tuple(state) != (True, False, False):
            raise RuntimeError("guest session authority check state differs")
    return check["sqltext"]


def _legacy_model(table, removed, check_name):
    metadata = MetaData()
    for candidate in Base.metadata.sorted_tables:
        candidate.to_metadata(metadata)
    old = metadata.tables[table.name]
    for constraint in list(old.constraints):
        if isinstance(constraint, CheckConstraint) and constraint.name == check_name:
            old.constraints.remove(constraint)
    for name in removed:
        old._columns.remove(old.c[name])
    if table.name == "browser_controllers":
        old.c.user_id.nullable = False
        old.c.user_session_version.nullable = False
    else:
        old.c.requester_user_id.nullable = False
    return old


def _rebuild_sqlite(connection, table, legacy):
    if connection.scalar(text("PRAGMA foreign_keys")):
        raise RuntimeError(
            "SQLite guest upgrade requires offline foreign key migration"
        )
    metadata = MetaData()
    for candidate in Base.metadata.sorted_tables:
        candidate.to_metadata(metadata)
    temporary = table.to_metadata(metadata, name=table.name + "_guest_upgrade")
    connection.execute(CreateTable(temporary))
    names = ",".join('"' + column.name + '"' for column in legacy.columns)
    connection.execute(
        text(
            'INSERT INTO "'
            + temporary.name
            + '" ('
            + names
            + ") SELECT "
            + names
            + ' FROM "'
            + table.name
            + '"'
        )
    )
    connection.execute(text('DROP TABLE "' + table.name + '"'))
    connection.execute(
        text('ALTER TABLE "' + temporary.name + '" RENAME TO "' + table.name + '"')
    )
    for index in table.indexes:
        index.create(connection)


def upgrade_browser_authority(connection):
    from app.db.migrate_add_browser_controllers import _verify_browser_principal_table

    columns = {
        c["name"] for c in inspect(connection).get_columns("browser_controllers")
    }
    if columns & BROWSER_GUEST_COLUMNS:
        if not BROWSER_GUEST_COLUMNS <= columns:
            raise RuntimeError(
                "browser principal schema differs: partial guest authority"
            )
        return
    legacy = _legacy_model(
        BrowserController.__table__, BROWSER_GUEST_COLUMNS, "ck_browser_authority"
    )
    _verify_browser_principal_table(connection, expected_table=legacy)
    if connection.dialect.name == "postgresql":
        connection.execute(
            text(
                "ALTER TABLE browser_controllers ADD COLUMN authority_kind VARCHAR(24) NOT NULL DEFAULT 'account', ADD COLUMN temporary_access_generation BIGINT NULL, ADD COLUMN target_auth_version BIGINT NULL, ALTER COLUMN user_id DROP NOT NULL, ALTER COLUMN user_session_version DROP NOT NULL"
            )
        )
        connection.execute(
            text(
                "ALTER TABLE browser_controllers ADD CONSTRAINT ck_browser_authority CHECK ("
                + expected_authority_check(BrowserController.__table__)
                + ")"
            )
        )
    elif connection.dialect.name == "sqlite":
        _rebuild_sqlite(connection, BrowserController.__table__, legacy)
    else:
        raise RuntimeError("guest authority migration requires PostgreSQL or SQLite")


def upgrade_session_authority(connection):
    inspector = inspect(connection)
    columns = {c["name"]: c for c in inspector.get_columns("session_requests")}
    if columns.keys() & SESSION_GUEST_COLUMNS:
        verify_guest_session_authority(connection)
        return
    legacy = _legacy_model(
        SessionRequest.__table__, SESSION_GUEST_COLUMNS, "ck_session_requests_authority"
    )
    if (
        set(columns) != set(legacy.c.keys())
        or columns["requester_user_id"]["nullable"] is not False
    ):
        raise RuntimeError("guest migration requires known account session schema")
    if connection.dialect.name == "postgresql":
        # The account migration owns its existing CHECKs, FKs and catalog states.
        # Verify them before allowing this migration to make user IDs nullable.
        from app.db.migrate_add_relay_access import _verify

        _verify(connection, None)
    else:
        from app.db.migrate_add_browser_controllers import (
            _verify_browser_principal_table,
        )

        _verify_browser_principal_table(connection, expected_table=legacy)
    if connection.dialect.name == "postgresql":
        connection.execute(
            text(
                "ALTER TABLE session_requests ADD COLUMN authority_kind VARCHAR(24) NOT NULL DEFAULT 'account', ADD COLUMN temporary_access_generation BIGINT NULL, ADD COLUMN target_auth_version BIGINT NULL, ADD COLUMN authority_expires_at TIMESTAMPTZ NULL, ALTER COLUMN requester_user_id DROP NOT NULL"
            )
        )
        connection.execute(
            text(
                "ALTER TABLE session_requests ADD CONSTRAINT ck_session_requests_authority CHECK ("
                + expected_authority_check(SessionRequest.__table__)
                + ")"
            )
        )
    elif connection.dialect.name == "sqlite":
        _rebuild_sqlite(connection, SessionRequest.__table__, legacy)
    else:
        raise RuntimeError("guest authority migration requires PostgreSQL or SQLite")
    verify_guest_session_authority(connection)


def migrate_connection(connection):
    from app.db.migrate_add_browser_controllers import _verify_browser_principal_table

    inspector = inspect(connection)
    if not inspector.has_table("session_requests") or not inspector.has_table(
        "browser_controllers"
    ):
        raise RuntimeError("guest migration requires existing session/browser schema")
    upgrade_session_authority(connection)
    upgrade_browser_authority(connection)
    _verify_browser_principal_table(connection)
    for table in (DeviceTemporaryAccess.__table__, GuestAccessAttempt.__table__):
        table.create(connection, checkfirst=True)
        _verify_browser_principal_table(connection, expected_table=table)


async def migrate(connection):
    if connection.dialect.name == "postgresql":
        await connection.execute(text("SELECT pg_advisory_xact_lock(672983001907)"))
    await connection.run_sync(migrate_connection)
