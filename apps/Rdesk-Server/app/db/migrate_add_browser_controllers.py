"""Add browser principals without changing existing device/session protocol data."""
import re
from sqlalchemy import String, inspect, text

from app.models.browser_controller import BrowserController


def verify_browser_device_column(connection, schema=None, *, inspector=None):
    """Return the exact owned CHECK after verifying its column and expression.

    IN is used by SQLite and model DDL; PostgreSQL reports it as = ANY.
    Both spellings describe the same closed two-value domain.
    """
    from app.db.migrate_add_relay_access import _CHECK_CAST, _normalize_server_default
    inspector = inspector or inspect(connection)
    columns = {column["name"]: column for column in inspector.get_columns("devices", schema=schema)}
    column = columns.get("principal_kind")
    checks = {check["name"]: check["sqltext"]
              for check in inspector.get_check_constraints("devices", schema=schema)}
    expression = checks.get("ck_devices_principal_kind")
    # Replace parentheses with token boundaries, rather than dropping whitespace:
    # a function named principal_kindin must not masquerade as SQL's IN operator.
    flattened = _CHECK_CAST.sub("", expression.lower()).replace('"', '').translate(
        str.maketrans({"(": " ", ")": " "})
    ) if isinstance(expression, str) else ""
    valid_expression = any(re.fullmatch(pattern, flattened) for pattern in (
        r"\s*principal_kind\s+in\s*'physical'\s*,\s*'browser_controller'\s*",
        r"\s*principal_kind\s*=\s*any\s+array\s*\[\s*'physical'\s*,\s*'browser_controller'\s*\]\s*",
    ))
    if (column is None or column["nullable"] is not False
        or not isinstance(column["type"], String) or column["type"].length != 24
        or _normalize_server_default(column["default"]) != "'physical'"
        or not valid_expression):
        raise RuntimeError("browser device principal schema differs")
    return expression


def _browser_check_expression(expression):
    from app.db.migrate_add_relay_access import _CHECK_CAST
    if not isinstance(expression, str):
        return None
    # PostgreSQL adds casts and parentheses to these four simple predicates.
    # Keep token boundaries so an altered function/operator cannot collapse into
    # the same expression; added boolean clauses remain visible and are rejected.
    normalized = _CHECK_CAST.sub("", expression.lower()).replace('"', '')
    normalized = normalized.translate(str.maketrans({"(": " ", ")": " "}))
    normalized = re.sub(r"(>=|<=|<>|!=|=|>|<)", r" \1 ", normalized)
    return " ".join(normalized.split())


def _verify_browser_principal_table(connection):
    from sqlalchemy import CheckConstraint, ForeignKeyConstraint, UniqueConstraint
    from app.db.migrate_add_relay_access import (
        _assert_constraint_states, _foreign_key_signature, _index_access_method,
        _primary_key_matches, _unique_constraint_signature,
    )

    table = BrowserController.__table__
    inspector = inspect(connection)
    postgresql = connection.dialect.name == "postgresql"
    current_schema = inspector.default_schema_name

    def differs(detail):
        raise RuntimeError("browser principal schema differs: " + detail)

    def constraint_name(constraint, suffix):
        if constraint.name is not None or not postgresql:
            return constraint.name
        return table.name + "_" + suffix

    expected_columns = {column.name: column for column in table.columns}
    columns = inspector.get_columns(table.name)
    actual_columns = {column["name"]: column for column in columns}
    if len(columns) != len(expected_columns) or set(actual_columns) != set(expected_columns):
        differs("columns")
    for name, column in expected_columns.items():
        actual = actual_columns[name]
        # Compare the physical dialect type: SQLite cannot reflect timezone or
        # BLOB length; PostgreSQL must retain TIMESTAMPTZ, BYTEA and JSONB.
        if (actual["nullable"] is not column.nullable
            or str(actual["type"].compile(dialect=connection.dialect)).upper()
                != str(column.type.compile(dialect=connection.dialect)).upper()
            or actual.get("default") is not None
            or actual.get("computed") is not None
            or actual.get("identity") is not None):
            differs("column " + name)

    primary_name = constraint_name(table.primary_key, "pkey")
    primary_key = inspector.get_pk_constraint(table.name)
    if (not _primary_key_matches(primary_key, name=primary_name,
            columns=tuple(column.name for column in table.primary_key.columns))
        or set(primary_key.get("dialect_options") or {}) - {"postgresql_include"}):
        differs("primary key")

    expected_checks = {constraint.name: _browser_check_expression(str(constraint.sqltext))
        for constraint in table.constraints if isinstance(constraint, CheckConstraint)}
    checks = inspector.get_check_constraints(table.name)
    if (len(checks) != len(expected_checks)
        or {check["name"]: _browser_check_expression(check["sqltext"])
            for check in checks} != expected_checks
        or any(set(check.get("dialect_options") or {}) - {"postgresql_not_valid"}
            or (check.get("dialect_options") or {}).get("postgresql_not_valid", False)
            for check in checks)):
        differs("checks")

    expected_unique = {
        (constraint_name(constraint, "_".join(column.name for column in constraint.columns) + "_key"),
            tuple(column.name for column in constraint.columns), False, (), ())
        for constraint in table.constraints if isinstance(constraint, UniqueConstraint)}
    unique = inspector.get_unique_constraints(table.name)
    if (len(unique) != len(expected_unique)
        or {_unique_constraint_signature(item) for item in unique} != expected_unique
        or any(set(item.get("dialect_options") or {})
            - {"postgresql_include", "postgresql_nulls_not_distinct"} for item in unique)):
        differs("unique constraints")

    expected_foreign_keys = set()
    for constraint in table.constraints:
        if not isinstance(constraint, ForeignKeyConstraint):
            continue
        elements = list(constraint.elements)
        expected_foreign_keys.add((
            constraint_name(constraint, "_".join(element.parent.name for element in elements) + "_fkey"),
            tuple(element.parent.name for element in elements),
            elements[0].column.table.schema or current_schema,
            elements[0].column.table.name,
            tuple(element.column.name for element in elements),
            "CASCADE", "NO ACTION", False, None, "SIMPLE",
        ))
    foreign_keys = inspector.get_foreign_keys(table.name)
    if (len(foreign_keys) != len(expected_foreign_keys)
        or {_foreign_key_signature(key, current_schema=current_schema)
            for key in foreign_keys} != expected_foreign_keys
        or any(set(key.get("options") or {})
            - {"ondelete", "onupdate", "deferrable", "initially", "match"}
            for key in foreign_keys)):
        differs("foreign keys")

    if postgresql:
        try:
            _assert_constraint_states(connection, schema=current_schema, table_name=table.name,
                expected_types={primary_name: "p", **{name: "c" for name in expected_checks},
                    **{signature[0]: "u" for signature in expected_unique},
                    **{signature[0]: "f" for signature in expected_foreign_keys}}, exact=True)
        except RuntimeError as error:
            raise RuntimeError("browser principal schema differs: constraint states") from error

    expected_indexes = {index.name: (tuple(column.name for column in index.columns), bool(index.unique))
        for index in table.indexes}
    indexes = inspector.get_indexes(table.name)
    # PostgreSQL reflects UNIQUE supporting indexes alongside standalone indexes.
    # Accept only the exact already-verified UNIQUE contract's supporting index.
    standalone = []
    for index in indexes:
        duplicate = index.get("duplicates_constraint")
        if duplicate:
            if not any(duplicate == signature[0]
                and tuple(index.get("column_names") or ()) == signature[1]
                and bool(index.get("unique")) for signature in expected_unique):
                differs("supporting indexes")
        else:
            standalone.append(index)
    actual_indexes = {index["name"]: index for index in standalone}
    if len(standalone) != len(expected_indexes) or set(actual_indexes) != set(expected_indexes):
        differs("indexes")
    for name, (index_columns, unique_index) in expected_indexes.items():
        index = actual_indexes[name]
        options = index.get("dialect_options") or {}
        if (tuple(index.get("column_names") or ()) != index_columns
            or bool(index.get("unique")) != unique_index
            or index.get("expressions") or index.get("include_columns")
            or index.get("column_sorting")
            or set(options) - {"postgresql_include", "postgresql_nulls_not_distinct", "postgresql_using"}
            or options.get("postgresql_include") not in (None, [])
            or options.get("postgresql_nulls_not_distinct") not in (None, False)
            or options.get("postgresql_using") not in (None, "btree")
            or (postgresql and _index_access_method(connection, schema=current_schema,
                index_name=name) != "btree")):
            differs("index " + name)


def migrate_connection(connection):
    inspector = inspect(connection)
    if not inspector.has_table("devices"):
        raise RuntimeError("browser migration requires the existing device schema")
    columns = {column["name"]: column for column in inspector.get_columns("devices")}
    if "principal_kind" not in columns:
        connection.execute(text("ALTER TABLE devices ADD COLUMN principal_kind VARCHAR(24) "
            "NOT NULL DEFAULT 'physical' CONSTRAINT ck_devices_principal_kind "
            "CHECK (principal_kind IN ('physical', 'browser_controller'))"))
    verify_browser_device_column(connection)
    BrowserController.__table__.create(connection, checkfirst=True)
    _verify_browser_principal_table(connection)


async def migrate(connection):
    if connection.dialect.name == "postgresql":
        await connection.execute(text("SELECT pg_advisory_xact_lock(672983001906)"))
    await connection.run_sync(migrate_connection)
