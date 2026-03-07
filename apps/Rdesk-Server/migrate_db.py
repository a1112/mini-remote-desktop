import asyncio
from sqlalchemy import text
from app.db.session import engine


async def migrate_users_table():
    """Add missing columns to users table."""

    async with engine.begin() as conn:
        # Add avatar_url column
        await conn.execute(text("""
            ALTER TABLE users
            ADD COLUMN IF NOT EXISTS avatar_url TEXT
        """))

        # Add created_at column
        await conn.execute(text("""
            ALTER TABLE users
            ADD COLUMN IF NOT EXISTS created_at TIMESTAMP
        """))

        # Add updated_at column
        await conn.execute(text("""
            ALTER TABLE users
            ADD COLUMN IF NOT EXISTS updated_at TIMESTAMP
        """))

        # Set default values for existing rows
        await conn.execute(text("""
            UPDATE users
            SET created_at = COALESCE(created_at, CURRENT_TIMESTAMP),
                updated_at = COALESCE(updated_at, CURRENT_TIMESTAMP)
        """))

        print("Migration completed successfully!")


if __name__ == "__main__":
    asyncio.run(migrate_users_table())
