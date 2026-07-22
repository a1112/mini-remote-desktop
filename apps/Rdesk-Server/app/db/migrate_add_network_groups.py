"""
数据库迁移脚本：添加网络分组功能

创建 network_groups 和 device_network_groups 表
"""
import asyncio
from sqlalchemy import text
from app.db.session import engine


async def migrate():
    """执行数据库迁移"""
    async with engine.begin() as conn:
        # 创建 network_groups 表
        await conn.execute(text("""
            CREATE TABLE IF NOT EXISTS network_groups (
                id VARCHAR(36) PRIMARY KEY,
                user_id VARCHAR(36) NOT NULL REFERENCES users(id),
                name VARCHAR(128) NOT NULL,
                description TEXT,
                is_enabled BOOLEAN DEFAULT TRUE,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
        """))

        # 创建索引
        await conn.execute(text("""
            CREATE INDEX IF NOT EXISTS idx_network_groups_user_id ON network_groups(user_id);
        """))

        # 创建 device_network_groups 表
        await conn.execute(text("""
            CREATE TABLE IF NOT EXISTS device_network_groups (
                id VARCHAR(36) PRIMARY KEY,
                network_group_id VARCHAR(36) NOT NULL REFERENCES network_groups(id) ON DELETE CASCADE,
                device_id VARCHAR(36) NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
                is_enabled BOOLEAN DEFAULT TRUE,
                assigned_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(network_group_id, device_id)
            );
        """))

        # 创建索引
        await conn.execute(text("""
            CREATE INDEX IF NOT EXISTS idx_device_network_groups_group_id ON device_network_groups(network_group_id);
        """))
        await conn.execute(text("""
            CREATE INDEX IF NOT EXISTS idx_device_network_groups_device_id ON device_network_groups(device_id);
        """))

        print("✓ 网络分组表创建成功")


async def rollback():
    """回滚迁移（删除表）"""
    async with engine.begin() as conn:
        await conn.execute(text("DROP TABLE IF EXISTS device_network_groups;"))
        await conn.execute(text("DROP TABLE IF EXISTS network_groups;"))
        print("✓ 网络分组表已删除")


if __name__ == "__main__":
    import sys

    if len(sys.argv) > 1 and sys.argv[1] == "rollback":
        print("正在回滚数据库迁移...")
        asyncio.run(rollback())
    else:
        print("正在执行数据库迁移...")
        asyncio.run(migrate())
