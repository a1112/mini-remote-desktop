import asyncio
from sqlalchemy import text
from app.db.session import engine


async def migrate_devices_table():
    """Add device binding fields to devices table."""

    async with engine.begin() as conn:
        # 添加设备绑定相关字段
        await conn.execute(text("""
            ALTER TABLE devices
            ADD COLUMN IF NOT EXISTS motherboard_serial VARCHAR(128) UNIQUE
        """))

        await conn.execute(text("""
            ALTER TABLE devices
            ADD COLUMN IF NOT EXISTS hostname VARCHAR(128)
        """))

        await conn.execute(text("""
            ALTER TABLE devices
            ADD COLUMN IF NOT EXISTS os_version TEXT
        """))

        await conn.execute(text("""
            ALTER TABLE devices
            ADD COLUMN IF NOT EXISTS cpu_info TEXT
        """))

        await conn.execute(text("""
            ALTER TABLE devices
            ADD COLUMN IF NOT EXISTS total_memory_mb INTEGER
        """))

        await conn.execute(text("""
            ALTER TABLE devices
            ADD COLUMN IF NOT EXISTS gpu_info TEXT
        """))

        await conn.execute(text("""
            ALTER TABLE devices
            ADD COLUMN IF NOT EXISTS is_bound BOOLEAN DEFAULT FALSE
        """))

        await conn.execute(text("""
            ALTER TABLE devices
            ADD COLUMN IF NOT EXISTS bound_at TIMESTAMP
        """))

        await conn.execute(text("""
            ALTER TABLE devices
            ADD COLUMN IF NOT EXISTS bound_user_id VARCHAR(36)
        """))

        # 为 motherboard_serial 创建索引（如果尚未存在）
        await conn.execute(text("""
            CREATE INDEX IF NOT EXISTS ix_devices_motherboard_serial
            ON devices(motherboard_serial)
        """))

        print("Device fields migration completed successfully!")


if __name__ == "__main__":
    asyncio.run(migrate_devices_table())
