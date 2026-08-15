from contextlib import asynccontextmanager
import shlex

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
import uvicorn

from app.api.v1.router import api_router
from app.core.config import settings
from app.db.init_db import seed_initial_data
from app.db.session import AsyncSessionLocal, Base, engine
from app.services.realtime_manager import RealtimeSidecarManager
import app.models  # noqa: F401


@asynccontextmanager
async def lifespan(_: FastAPI):
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
    async with AsyncSessionLocal() as db:
        await seed_initial_data(db)
    yield


app = FastAPI(title="Rdesk-Server", version="0.1.0", lifespan=lifespan)
app.state.realtime_manager = RealtimeSidecarManager(
    health_url=settings.realtime_server_health_url,
    command=[settings.realtime_server_command, *shlex.split(settings.realtime_server_args)],
    workdir=settings.realtime_server_workdir,
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=[item.strip() for item in settings.cors_origins.split(",") if item.strip()],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(api_router)


@app.get("/healthz")
async def healthz():
    return {"status": "ok"}


if __name__ == "__main__":
    uvicorn.run(
        "app.main:app",
        host=settings.server_host,
        port=settings.server_port,
        reload=settings.development_reload,
        workers=1,
        reload_dirs=["app"] if settings.development_reload else None,
    )
