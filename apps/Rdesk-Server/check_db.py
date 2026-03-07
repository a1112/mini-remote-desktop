import asyncio
from sqlalchemy import select
from app.db.session import AsyncSessionLocal, Base, engine
from app.core.security import hash_password
from app.models.user import User
import app.models  # noqa: F401


async def check_and_init_db():
    """Check database and create initial user if needed."""

    # Create tables
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)

    # Check for existing users
    async with AsyncSessionLocal() as db:
        result = await db.execute(select(User).limit(1))
        user = result.scalar_one_or_none()

        if user:
            print(f"User exists: {user.username} (id={user.id}, role={user.role})")
        else:
            print("No users found. Creating admin user...")
            admin = User(
                username="admin",
                email="admin@rdesk.local",
                password_hash=hash_password("admin123"),
                role="admin",
            )
            db.add(admin)
            await db.commit()
            await db.refresh(admin)
            print(f"Created admin user: {admin.username} (id={admin.id})")


if __name__ == "__main__":
    asyncio.run(check_and_init_db())
